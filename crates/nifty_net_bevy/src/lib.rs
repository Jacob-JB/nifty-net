
pub mod net_socket;
pub mod typed;

pub mod prelude {
    pub use nifty_net::Config;

    pub use crate::net_socket::{
        NetSocket,
        Connection,
        NetSocketConfig,
        NetworkingPlugin,
        Connected,
        Disconnected,
        FailedConnection,
        UpdateSockets,
    };

    pub use crate::typed::{
        TypedMessagePlugin,
        TypedMessages,
        Connections,
    };
}
