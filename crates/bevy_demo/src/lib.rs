
use nifty_net_bevy::prelude::*;
use serde::{Serialize, Deserialize};

pub fn typed_plugin() -> TypedMessagePlugin {
    TypedMessagePlugin::default()
    .with_message::<Ping>()
    .with_message::<Pong>()
}


#[derive(Serialize, Deserialize)]
pub struct Ping {
    pub message: String,
}

#[derive(Serialize, Deserialize)]
pub struct Pong {
    pub message: String,
}
