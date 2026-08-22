//! Public, surface-neutral contracts for conversational MCP mini apps.

mod compiler;
mod payment;
mod types;

pub use compiler::CompiledContent;
pub use compiler::ContentCompiler;
pub use compiler::ContentSource;
pub use compiler::SourceKind;
pub use payment::CheckoutStatus;
pub use payment::CreatePaymentRequest;
pub use payment::OpenCheckoutRequest;
pub use payment::PAYMENT_SCHEMA;
pub use payment::PaymentContractError;
pub use payment::PaymentStatusRequest;
pub use payment::PaymentView;
pub use types::AppSummary;
pub use types::ChatDisposition;
pub use types::ContentReceipt;
pub use types::Feed;
pub use types::FeedItem;
pub use types::FeedItemKind;
pub use types::HOME_SCHEMA;
pub use types::HomeDocument;
pub use types::HomeRequest;
pub use types::MAX_HOME_BYTES;
pub use types::MAX_HOME_FEED_ITEMS;
pub use types::QuickAction;
pub use types::QuickReply;
pub use types::RichMessage;
pub use types::Tip;
pub use types::Welcome;
pub use types::extract_structured_content;
pub use types::mcp_result_text;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
