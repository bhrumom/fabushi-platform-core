use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use fabushi_telegram_network::{establish_auth_key, NetworkConfig, TcpTransport};
use fabushi_telegram_protocol::{
    DcDirectory, ObfuscatedConnectionStart, PlaintextEnvelope, TransportFrameCodec, TransportMode,
};

#[test]
fn abridged_transport_handles_fragmented_tcp_reads_and_writes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut header = [0_u8; 1];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header, [0xef]);

        let mut length = [0_u8; 1];
        stream.read_exact(&mut length).unwrap();
        let body_length = usize::from(length[0]) * 4;
        let mut request = vec![0_u8; body_length];
        stream.read_exact(&mut request).unwrap();
        assert_eq!(request, vec![0x41; 12]);

        let mut codec = TransportFrameCodec::new(TransportMode::Abridged);
        let response = codec.encode(&[0x42; 16]).unwrap();
        for fragment in response.chunks(3) {
            stream.write_all(fragment).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
    });

    let stream = TcpStream::connect(address).unwrap();
    let mut transport = TcpTransport::from_stream(stream, TransportMode::Abridged);
    transport.send_payload(&[0x41; 12]).unwrap();
    assert_eq!(transport.receive_payload().unwrap(), vec![0x42; 16]);
    server.join().unwrap();
}

#[test]
fn plaintext_exchange_preserves_wire_envelope() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut transport = TcpTransport::from_stream(stream, TransportMode::Intermediate);
        let payload = transport.receive_payload().unwrap();
        let request = PlaintextEnvelope::decode(&payload).unwrap();
        assert_eq!(request.body, vec![1, 2, 3, 4]);
        transport
            .send_payload(
                &PlaintextEnvelope {
                    message_id: 9,
                    body: vec![5, 6, 7, 8],
                }
                .encode()
                .unwrap(),
            )
            .unwrap();
    });

    let stream = TcpStream::connect(address).unwrap();
    let mut transport = TcpTransport::from_stream(stream, TransportMode::Intermediate);
    let response = transport
        .exchange_plaintext(&PlaintextEnvelope {
            message_id: 4,
            body: vec![1, 2, 3, 4],
        })
        .unwrap();
    assert_eq!(response.message_id, 9);
    assert_eq!(response.body, vec![5, 6, 7, 8]);
    server.join().unwrap();
}

#[test]
#[ignore = "requires a live Telegram production data center"]
fn live_session_exchanges_encrypted_ping_and_service_rpc() {
    let directory = DcDirectory::telegram_defaults(false, 2).unwrap();
    let mut session = establish_auth_key(&directory, 2, &NetworkConfig::default()).unwrap();
    session.ping().unwrap();

    let mut request = Vec::with_capacity(8);
    request.extend_from_slice(&0xb921_bd04_u32.to_le_bytes());
    request.extend_from_slice(&1_i32.to_le_bytes());
    let result = session.invoke_raw(&request).unwrap();
    assert_eq!(
        u32::from_le_bytes(result.body[..4].try_into().unwrap()),
        0xae50_0895
    );
}


#[test]
fn obfuscated_tcp_transport_writes_prefix_and_keeps_bidirectional_ctr_streams_contiguous() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        let mut prefix = [0_u8; 64];
        stream.read_exact(&mut prefix).unwrap();
        assert!(fabushi_telegram_protocol::is_good_direct_start_nonce(
            &prefix
        ));

        // Fixed-U leaves nonce[0..56] plaintext. The remaining random tail does
        // not participate in key/IV derivation, so any bytes there can rebuild
        // the same post-prefix send/receive stream positions for this fixture.
        let mut reconstructed_nonce = [0x55_u8; 64];
        reconstructed_nonce[..56].copy_from_slice(&prefix[..56]);
        let mut peer = ObfuscatedConnectionStart::from_nonce(&[], 2, reconstructed_nonce).unwrap();

        let mut encrypted_request = [0_u8; 13];
        stream.read_exact(&mut encrypted_request).unwrap();
        peer.encrypt_outbound(&mut encrypted_request);
        assert_eq!(encrypted_request[0], 3);
        assert_eq!(&encrypted_request[1..], &[0x41; 12]);

        let mut response_codec = TransportFrameCodec::new(TransportMode::Abridged);
        let mut response = response_codec.encode(&[0x42; 16]).unwrap();
        peer.decrypt_inbound(&mut response);
        for fragment in response.chunks(3) {
            stream.write_all(fragment).unwrap();
        }
    });

    let config = NetworkConfig::default();
    let mut transport = TcpTransport::connect_obfuscated(address, &[], 2, &config).unwrap();
    transport.send_payload(&[0x41; 12]).unwrap();
    assert_eq!(transport.receive_payload().unwrap(), vec![0x42; 16]);
    server.join().unwrap();
}
