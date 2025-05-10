pub mod config;
mod connection;
pub mod error;
pub mod sdk;
pub mod types;

pub use config::{SDKConfig, TunnelConfig};
pub use error::TunnelError;
pub use sdk::TunnelClient;

pub use types::{TunnelMessage, TunnelMessageType, TunnelStatus};

pub mod prelude {
    pub use crate::config::{
        OnAuthCallback, OnConnectedCallback, OnDisconnectedCallback, OnErrorCallback,
        OnRequestCallback, OnSendingResponseCallback, SDKConfig, TunnelConfig,
    };
    pub use crate::error::TunnelError;
    pub use crate::sdk::TunnelClient;
    pub use crate::types::{TunnelMessage, TunnelMessageType, TunnelStatus};
}
