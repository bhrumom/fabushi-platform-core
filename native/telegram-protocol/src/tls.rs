use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;
use x25519_dalek::{x25519, X25519_BASEPOINT_BYTES};

const TLS_MARKER: u8 = 0xee;
const TLS_KEY_LENGTH: usize = 16;
const TLS_SECRET_PREFIX_LENGTH: usize = 17;
const MAX_GREASES: usize = 8;
const CLIENT_HELLO_LIMIT: usize = 2048;
const HELLO_DIGEST_LENGTH: usize = 32;
const LENGTH_SIZE: usize = 2;
const MAX_SERVER_HELLO_LENGTH: usize = 65_536;
const SERVER_HELLO_PART1: &[u8] = b"\x16\x03\x03";
const SERVER_HELLO_PART3: &[u8] = b"\x14\x03\x03\x00\x01\x01\x17\x03\x03";
const SERVER_HELLO_DIGEST_POSITION: usize = 11;
const SERVER_RECORD_HEADER: &[u8] = b"\x17\x03\x03";
const CLIENT_PART_SIZE: usize = 2878;
const CLIENT_PREFIX: &[u8] = b"\x14\x03\x03\x00\x01\x01";
const CLIENT_RECORD_HEADER: &[u8] = b"\x17\x03\x03";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TlsCamouflageError {
    #[error("invalid MTProto TLS camouflage secret")]
    InvalidSecret,
    #[error("operating system secure randomness is unavailable")]
    RandomnessUnavailable,
    #[error("system clock is earlier than the Unix epoch")]
    InvalidSystemClock,
    #[error("TLS camouflage template exceeds the fixed-U ClientHello limit")]
    ClientHelloLimit,
    #[error("TLS camouflage scoped block length cannot be represented on the wire")]
    ScopeLengthOutOfRange,
    #[error("TLS camouflage template does not contain the digest slot")]
    MissingDigestSlot,
    #[error("TLS camouflage GREASE seed is invalid")]
    InvalidGreaseSeed,
    #[error("TLS camouflage server hello is malformed")]
    InvalidServerHello,
    #[error("TLS camouflage server hello authentication failed")]
    ServerHelloAuthenticationFailed,
    #[error("TLS camouflage application record is malformed")]
    InvalidRecord,
    #[error("TLS camouflage client record prefix exceeds the fixed-U payload limit")]
    ClientRecordPrefixTooLarge,
    #[error("TLS camouflage application payload must not be empty")]
    EmptyApplicationPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsCamouflageSecret {
    key: [u8; TLS_KEY_LENGTH],
    domain: Vec<u8>,
}

impl TlsCamouflageSecret {
    pub fn parse(secret: &[u8]) -> Result<Self, TlsCamouflageError> {
        if secret.len() < 21 || secret.first().copied() != Some(TLS_MARKER) {
            return Err(TlsCamouflageError::InvalidSecret);
        }
        Ok(Self {
            key: secret[1..TLS_SECRET_PREFIX_LENGTH]
                .try_into()
                .expect("sixteen-byte TLS camouflage key"),
            domain: secret[TLS_SECRET_PREFIX_LENGTH..].to_vec(),
        })
    }

    pub fn key(&self) -> &[u8; TLS_KEY_LENGTH] {
        &self.key
    }

    pub fn domain(&self) -> &[u8] {
        &self.domain
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsClientHello {
    pub bytes: Vec<u8>,
    pub digest: [u8; HELLO_DIGEST_LENGTH],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedTlsRecord {
    Empty { consumed_bytes: usize },
    Payload {
        payload: Vec<u8>,
        consumed_bytes: usize,
    },
}

#[derive(Debug, Clone)]
enum TlsBlock {
    String(&'static [u8]),
    Random(usize),
    Zero(usize),
    Domain,
    Grease(usize),
    PublicKey,
    Scope(Vec<TlsBlock>),
    Permutation(Vec<Vec<TlsBlock>>),
    M,
    E,
    Padding,
}

struct Part<'a> {
    domain: &'a [u8],
    greases: &'a [u8; MAX_GREASES],
    result: Vec<u8>,
    digest_position: Option<usize>,
}

impl<'a> Part<'a> {
    fn new(domain: &'a [u8], greases: &'a [u8; MAX_GREASES]) -> Self {
        Self {
            domain,
            greases,
            result: Vec::with_capacity(CLIENT_HELLO_LIMIT),
            digest_position: None,
        }
    }

    fn grow(&mut self, size: usize) -> Result<usize, TlsCamouflageError> {
        if size == 0 || self.result.len().saturating_add(size) > CLIENT_HELLO_LIMIT {
            return Err(TlsCamouflageError::ClientHelloLimit);
        }
        let offset = self.result.len();
        self.result.resize(offset + size, 0);
        Ok(offset)
    }

    fn write_blocks(
        &mut self,
        blocks: &[TlsBlock],
        random: &mut dyn FnMut(&mut [u8]) -> Result<(), TlsCamouflageError>,
    ) -> Result<(), TlsCamouflageError> {
        for block in blocks {
            self.write_block(block, random)?;
        }
        Ok(())
    }

    fn write_block(
        &mut self,
        block: &TlsBlock,
        random: &mut dyn FnMut(&mut [u8]) -> Result<(), TlsCamouflageError>,
    ) -> Result<(), TlsCamouflageError> {
        match block {
            TlsBlock::String(bytes) => {
                let offset = self.grow(bytes.len())?;
                self.result[offset..offset + bytes.len()].copy_from_slice(bytes);
            }
            TlsBlock::Zero(length) => {
                let offset = self.grow(*length)?;
                if *length == HELLO_DIGEST_LENGTH && self.digest_position.is_none() {
                    self.digest_position = Some(offset);
                }
            }
            TlsBlock::Grease(seed) => {
                let grease = *self
                    .greases
                    .get(*seed)
                    .ok_or(TlsCamouflageError::InvalidGreaseSeed)?;
                let offset = self.grow(2)?;
                self.result[offset..offset + 2].fill(grease);
            }
            TlsBlock::Random(length) => {
                let offset = self.grow(*length)?;
                random(&mut self.result[offset..offset + *length])?;
            }
            TlsBlock::Domain => {
                let offset = self.grow(self.domain.len())?;
                self.result[offset..offset + self.domain.len()].copy_from_slice(self.domain);
            }
            TlsBlock::PublicKey => {
                let mut scalar = [0_u8; 32];
                random(&mut scalar)?;
                let public_key = x25519(scalar, X25519_BASEPOINT_BYTES);
                let offset = self.grow(public_key.len())?;
                self.result[offset..offset + public_key.len()].copy_from_slice(&public_key);
            }
            TlsBlock::Scope(entries) => {
                let length_offset = self.grow(LENGTH_SIZE)?;
                let payload_offset = self.result.len();
                self.write_blocks(entries, random)?;
                let length = self.result.len() - payload_offset;
                let length: u16 = length
                    .try_into()
                    .map_err(|_| TlsCamouflageError::ScopeLengthOutOfRange)?;
                self.result[length_offset..length_offset + LENGTH_SIZE]
                    .copy_from_slice(&length.to_be_bytes());
            }
            TlsBlock::Permutation(entries) => {
                let mut rendered = Vec::with_capacity(entries.len());
                for entries in entries {
                    let mut part = Part::new(self.domain, self.greases);
                    part.write_blocks(entries, random)?;
                    rendered.push(part.result);
                }
                shuffle(&mut rendered, random)?;
                for element in rendered {
                    let offset = self.grow(element.len())?;
                    self.result[offset..offset + element.len()].copy_from_slice(&element);
                }
            }
            TlsBlock::M => {
                const ELEMENTS: usize = 384;
                const ADDED: usize = 32;
                let offset = self.grow(ELEMENTS * 3 + ADDED)?;
                let mut source = vec![0_u8; ELEMENTS * 8 + ADDED];
                random(&mut source)?;
                for i in 0..ELEMENTS {
                    let pair = i * 8;
                    let a = u32::from_le_bytes(source[pair..pair + 4].try_into().expect("four bytes"))
                        % 3329;
                    let b = u32::from_le_bytes(
                        source[pair + 4..pair + 8]
                            .try_into()
                            .expect("four bytes"),
                    ) % 3329;
                    let target = offset + i * 3;
                    self.result[target] = (a & 0xff) as u8;
                    self.result[target + 1] = ((a >> 8) + ((b & 0x0f) << 4)) as u8;
                    self.result[target + 2] = (b >> 4) as u8;
                }
                random(&mut self.result[offset + ELEMENTS * 3..offset + ELEMENTS * 3 + ADDED])?;
            }
            TlsBlock::E => {
                const LENGTHS: [usize; 4] = [144, 176, 208, 240];
                let index = random_index(LENGTHS.len(), random)?;
                self.write_block(&TlsBlock::Random(LENGTHS[index]), random)?;
            }
            TlsBlock::Padding => {
                let length = self.result.len();
                if length < 513 {
                    let zero = TlsBlock::Zero(513 - length);
                    self.write_block(&TlsBlock::String(b"\x00\x15"), random)?;
                    self.write_block(&TlsBlock::Scope(vec![zero]), random)?;
                }
            }
        }
        Ok(())
    }

    fn finalize(
        mut self,
        key: &[u8; TLS_KEY_LENGTH],
        timestamp: i32,
    ) -> Result<TlsClientHello, TlsCamouflageError> {
        let position = self
            .digest_position
            .ok_or(TlsCamouflageError::MissingDigestSlot)?;
        let digest = tls_camouflage_hmac(key, &self.result);
        self.result[position..position + HELLO_DIGEST_LENGTH].copy_from_slice(&digest);

        let timestamp_offset = position + HELLO_DIGEST_LENGTH - 4;
        let mut tail = u32::from_le_bytes(
            self.result[timestamp_offset..timestamp_offset + 4]
                .try_into()
                .expect("four-byte digest tail"),
        );
        tail ^= timestamp as u32;
        self.result[timestamp_offset..timestamp_offset + 4]
            .copy_from_slice(&tail.to_le_bytes());

        let digest = self.result[position..position + HELLO_DIGEST_LENGTH]
            .try_into()
            .expect("thirty-two-byte digest");
        Ok(TlsClientHello {
            bytes: self.result,
            digest,
        })
    }
}

pub fn tls_camouflage_hmac(
    key: &[u8; TLS_KEY_LENGTH],
    data: &[u8],
) -> [u8; HELLO_DIGEST_LENGTH] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a sixteen-byte key");
    mac.update(data);
    let bytes = mac.finalize().into_bytes();
    let mut output = [0_u8; HELLO_DIGEST_LENGTH];
    output.copy_from_slice(&bytes);
    output
}

pub fn prepare_tls_client_hello(secret: &[u8]) -> Result<TlsClientHello, TlsCamouflageError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TlsCamouflageError::InvalidSystemClock)?
        .as_secs() as i32;
    prepare_tls_client_hello_with_entropy(secret, timestamp, &mut |output| {
        getrandom::getrandom(output).map_err(|_| TlsCamouflageError::RandomnessUnavailable)
    })
}

pub fn prepare_tls_client_hello_with_entropy(
    secret: &[u8],
    timestamp: i32,
    random: &mut dyn FnMut(&mut [u8]) -> Result<(), TlsCamouflageError>,
) -> Result<TlsClientHello, TlsCamouflageError> {
    let secret = TlsCamouflageSecret::parse(secret)?;
    let mut greases = [0_u8; MAX_GREASES];
    random(&mut greases)?;
    for grease in &mut greases {
        *grease = (*grease & 0xf0) + 0x0a;
    }
    for pair in greases.chunks_exact_mut(2) {
        if pair[0] == pair[1] {
            pair[1] ^= 0x10;
        }
    }

    let rules = prepare_client_hello_rules();
    let mut part = Part::new(secret.domain(), &greases);
    part.write_blocks(&rules, random)?;
    part.finalize(secret.key(), timestamp)
}

pub fn inspect_tls_server_hello(
    secret: &[u8],
    client_digest: &[u8; HELLO_DIGEST_LENGTH],
    input: &[u8],
) -> Result<Option<usize>, TlsCamouflageError> {
    let secret = TlsCamouflageSecret::parse(secret)?;
    let part1_size = SERVER_HELLO_PART1.len() + LENGTH_SIZE;
    if input.len() < part1_size {
        return Ok(None);
    }
    if !input.starts_with(SERVER_HELLO_PART1) {
        return Err(TlsCamouflageError::InvalidServerHello);
    }
    let part2_size = u16::from_be_bytes(
        input[SERVER_HELLO_PART1.len()..part1_size]
            .try_into()
            .expect("two-byte ServerHello length"),
    ) as usize;
    let parts123_size = part1_size
        .checked_add(part2_size)
        .and_then(|value| value.checked_add(SERVER_HELLO_PART3.len() + LENGTH_SIZE))
        .ok_or(TlsCamouflageError::InvalidServerHello)?;
    if parts123_size > MAX_SERVER_HELLO_LENGTH {
        return Err(TlsCamouflageError::InvalidServerHello);
    }
    if input.len() < parts123_size {
        return Ok(None);
    }
    let part3_offset = parts123_size - LENGTH_SIZE - SERVER_HELLO_PART3.len();
    if input.get(part3_offset..part3_offset + SERVER_HELLO_PART3.len())
        != Some(SERVER_HELLO_PART3)
    {
        return Err(TlsCamouflageError::InvalidServerHello);
    }
    let part4_size = u16::from_be_bytes(
        input[parts123_size - LENGTH_SIZE..parts123_size]
            .try_into()
            .expect("two-byte application length"),
    ) as usize;
    let full = parts123_size
        .checked_add(part4_size)
        .ok_or(TlsCamouflageError::InvalidServerHello)?;
    if full > MAX_SERVER_HELLO_LENGTH {
        return Err(TlsCamouflageError::InvalidServerHello);
    }
    if input.len() < full {
        return Ok(None);
    }
    if full < SERVER_HELLO_DIGEST_POSITION + HELLO_DIGEST_LENGTH {
        return Err(TlsCamouflageError::InvalidServerHello);
    }

    let mut authenticated = Vec::with_capacity(HELLO_DIGEST_LENGTH + full);
    authenticated.extend_from_slice(client_digest);
    authenticated.extend_from_slice(&input[..full]);
    let digest_offset = HELLO_DIGEST_LENGTH + SERVER_HELLO_DIGEST_POSITION;
    let digest: [u8; HELLO_DIGEST_LENGTH] = authenticated
        [digest_offset..digest_offset + HELLO_DIGEST_LENGTH]
        .try_into()
        .expect("thirty-two-byte ServerHello digest");
    authenticated[digest_offset..digest_offset + HELLO_DIGEST_LENGTH].fill(0);
    let expected = tls_camouflage_hmac(secret.key(), &authenticated);
    if !bool::from(digest.ct_eq(&expected)) {
        return Err(TlsCamouflageError::ServerHelloAuthenticationFailed);
    }
    Ok(Some(full))
}

pub fn encode_tls_client_records(
    prefix: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, TlsCamouflageError> {
    if data.is_empty() {
        return Err(TlsCamouflageError::EmptyApplicationPayload);
    }
    if prefix.len() > CLIENT_PART_SIZE {
        return Err(TlsCamouflageError::ClientRecordPrefixTooLarge);
    }
    let mut output = Vec::new();
    if !prefix.is_empty() {
        output.extend_from_slice(CLIENT_PREFIX);
    }
    let mut remaining = data;
    let mut first_prefix = prefix;
    while !remaining.is_empty() {
        let capacity = CLIENT_PART_SIZE - first_prefix.len();
        let write = capacity.min(remaining.len());
        let length = first_prefix.len() + write;
        let length: u16 = length
            .try_into()
            .map_err(|_| TlsCamouflageError::InvalidRecord)?;
        output.extend_from_slice(CLIENT_RECORD_HEADER);
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(first_prefix);
        output.extend_from_slice(&remaining[..write]);
        first_prefix = &[];
        remaining = &remaining[write..];
    }
    Ok(output)
}

pub fn decode_tls_server_record(
    input: &[u8],
) -> Result<Option<DecodedTlsRecord>, TlsCamouflageError> {
    let header_length = SERVER_RECORD_HEADER.len() + LENGTH_SIZE;
    if input.len() < header_length {
        return Ok(None);
    }
    if !input.starts_with(SERVER_RECORD_HEADER) {
        return Err(TlsCamouflageError::InvalidRecord);
    }
    let length = u16::from_be_bytes(
        input[SERVER_RECORD_HEADER.len()..header_length]
            .try_into()
            .expect("two-byte TLS record length"),
    ) as usize;
    let consumed_bytes = header_length
        .checked_add(length)
        .ok_or(TlsCamouflageError::InvalidRecord)?;
    if input.len() < consumed_bytes {
        return Ok(None);
    }
    if length == 0 {
        return Ok(Some(DecodedTlsRecord::Empty { consumed_bytes }));
    }
    Ok(Some(DecodedTlsRecord::Payload {
        payload: input[header_length..consumed_bytes].to_vec(),
        consumed_bytes,
    }))
}

fn shuffle(
    values: &mut [Vec<u8>],
    random: &mut dyn FnMut(&mut [u8]) -> Result<(), TlsCamouflageError>,
) -> Result<(), TlsCamouflageError> {
    for index in (1..values.len()).rev() {
        let selected = random_index(index + 1, random)?;
        values.swap(index, selected);
    }
    Ok(())
}

fn random_index(
    upper: usize,
    random: &mut dyn FnMut(&mut [u8]) -> Result<(), TlsCamouflageError>,
) -> Result<usize, TlsCamouflageError> {
    debug_assert!(upper > 0);
    let upper = upper as u64;
    let zone = u64::MAX - (u64::MAX % upper);
    loop {
        let mut bytes = [0_u8; 8];
        random(&mut bytes)?;
        let value = u64::from_le_bytes(bytes);
        if value < zone {
            return Ok((value % upper) as usize);
        }
    }
}

fn prepare_client_hello_rules() -> Vec<TlsBlock> {
    use TlsBlock::*;

    vec![
        String(b"\x16\x03\x01"),
        Scope(vec![
            String(b"\x01\x00"),
            Scope(vec![
                String(b"\x03\x03"),
                Zero(32),
                String(b"\x20"),
                Random(32),
                String(b"\x00\x20"),
                Grease(0),
                String(
                    b"\x13\x01\x13\x02\x13\x03\xc0\x2b\xc0\x2f\xc0\x2c\xc0\x30\xcc\xa9\xcc\xa8\xc0\x13\xc0\x14\x00\x9c\x00\x9d\x00\x2f\x00\x35\x01\x00",
                ),
                Scope(vec![
                    Grease(2),
                    String(b"\x00\x00"),
                    Permutation(vec![
                        vec![
                            String(b"\x00\x00"),
                            Scope(vec![Scope(vec![
                                String(b"\x00"),
                                Scope(vec![Domain]),
                            ])]),
                        ],
                        vec![String(b"\x00\x05\x00\x05\x01\x00\x00\x00\x00")],
                        vec![
                            String(b"\x00\x0a\x00\x0c\x00\x0a"),
                            Grease(4),
                            String(b"\x11\xec\x00\x1d\x00\x17\x00\x18"),
                        ],
                        vec![String(b"\x00\x0b\x00\x02\x01\x00")],
                        vec![String(
                            b"\x00\x0d\x00\x18\x00\x16\x09\x04\x09\x05\x09\x06\x04\x03\x08\x04\x04\x01\x05\x03\x08\x05\x05\x01\x08\x06\x06\x01",
                        )],
                        vec![String(
                            b"\x00\x10\x00\x0e\x00\x0c\x02\x68\x32\x08\x68\x74\x74\x70\x2f\x31\x2e\x31",
                        )],
                        vec![String(b"\x00\x12\x00\x00")],
                        vec![String(b"\x00\x17\x00\x00")],
                        vec![String(b"\x00\x1b\x00\x03\x02\x00\x02")],
                        vec![String(b"\x00\x23\x00\x00")],
                        vec![
                            String(b"\x00\x2b\x00\x07\x06"),
                            Grease(6),
                            String(b"\x03\x04\x03\x03"),
                        ],
                        vec![String(b"\x00\x2d\x00\x02\x01\x01")],
                        vec![
                            String(b"\x00\x33\x04\xef\x04\xed"),
                            Grease(4),
                            String(b"\x00\x01\x00\x11\xec\x04\xc0"),
                            M,
                            PublicKey,
                            String(b"\x00\x1d\x00\x20"),
                            PublicKey,
                        ],
                        vec![String(b"\x44\xcd\x00\x05\x00\x03\x02\x68\x32")],
                        vec![
                            String(b"\xfe\x0d"),
                            Scope(vec![
                                String(b"\x00\x00\x01\x00\x01"),
                                Random(1),
                                String(b"\x00\x20"),
                                PublicKey,
                                Scope(vec![E]),
                            ]),
                        ],
                        vec![String(b"\xff\x01\x00\x01\x00")],
                    ]),
                    Grease(3),
                    String(b"\x00\x01\x00"),
                    Padding,
                ]),
            ]),
        ]),
    ]
}
