
use std::collections::VecDeque;

use bevy::prelude::*;
use serde::{Serialize, Deserialize};

use crate::{net_socket::{
    Connection,
    UpdateSockets,
}, prelude::Connected};

/// typed messages are sent in this set in [PreUpdate]
#[derive(Hash, Debug, PartialEq, Eq, Clone, SystemSet)]
pub struct SendTypedMessages;

/// typed messages are received in this set in [PreUpdate]
#[derive(Hash, Debug, PartialEq, Eq, Clone, SystemSet)]
pub struct ReadTypedMessages;


/// replaces how messages are sent and received in the application
/// with a typed message system
///
/// instead of sending and receiving bytes you can send and receive messages
/// through [TypedMessages<T>]
///
/// create this plugin construct its [Default] value,
/// then add all the messages you intend to send with either
/// [add_message](TypedMessagePlugin::add_message) or
/// [with_message](TypedMessagePlugin::with_message).
///
/// the order you add the plugins in defines how they are serialized,
/// and should be the same for any apps that talk to each other.
/// to ensure that this is the case it is best done in a shared function
#[derive(Default)]
pub struct TypedMessagePlugin {
    /// a list of functions to call to add messages to the app
    messages: Vec<Box<dyn Fn(&mut App, u16) + Send + Sync + 'static>>,
}

impl TypedMessagePlugin {
    /// adds a message to the plugin
    pub fn add_message<T: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static>(&mut self) {
        self.messages.push(Box::new(build_message::<T>));
    }

    /// adds a message to the plugin
    pub fn with_message<T: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static>(mut self) -> Self {
        self.add_message::<T>();
        self
    }
}

impl Plugin for TypedMessagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BufferedMessages>();

        app.add_systems(PreUpdate, (
            insert_typed_connections.after(UpdateSockets),
            apply_deferred, // make sure connections have been inserted and marked as typed before reading messages
            buffer_messages.in_set(ReadTypedMessages),
        ).chain());

        for (i, build) in self.messages.iter().enumerate() {
            build(app, i as u16);
        }
    }
}

fn build_message<T: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static>(app: &mut App, message_id: u16) {
    app.insert_resource(TypedMessages::<T> {
        message_id,
        received: VecDeque::new(),
        send: VecDeque::new(),
    });

    app.add_systems(PreUpdate, (
        serialize_typed_messages::<T>.in_set(SendTypedMessages).before(UpdateSockets),
        deserialize_typed_messages::<T>.in_set(ReadTypedMessages).after(buffer_messages),
    ));
}


/// marker component that must be inserted onto a socket for it to gain typed message functionality
#[derive(Component)]
pub struct TypedSocket;

/// marker component that get's inserted onto clients that are children of [TypedSocket]s
#[derive(Component)]
struct TypedConnection;

fn insert_typed_connections(
    mut commands: Commands,
    mut connected_r: EventReader<Connected>,
    typed_socket_q: Query<(), With<TypedSocket>>,
) {
    for &Connected { socket_entity, connection_entity, .. } in connected_r.read() {
        if typed_socket_q.contains(socket_entity) {
            commands.entity(connection_entity).insert(TypedConnection);
        }
    }
}


#[derive(Resource, Default)]
struct BufferedMessages {
    messages: Vec<(Entity, Box<[u8]>)>,
}

fn buffer_messages(
    mut connection_q: Query<(Entity, &mut Connection), With<TypedConnection>>,
    mut buffer: ResMut<BufferedMessages>,
) {
    buffer.messages.clear();
    for (connection_entity, mut connection) in connection_q.iter_mut() {
        buffer.messages.extend(
            connection.drain_messages().map(|bytes| (connection_entity, bytes))
        );
    }
}


#[derive(Resource)]
pub struct TypedMessages<T> {
    message_id: u16,
    received: VecDeque<(Entity, T)>,
    send: VecDeque<(Entity, bool, Box<[u8]>)>,
}

/// runs after [buffer_messages] and deserializes messages into their appropriate [TypedMessages]
fn deserialize_typed_messages<T: for<'a> Deserialize<'a> + Send + Sync + 'static>(
    buffer: Res<BufferedMessages>,
    mut messages: ResMut<TypedMessages<T>>
) {
    messages.received.clear();

    for (connection_entity, bytes) in buffer.messages.iter() {
        let Some(message_id) = bytes.get(0..2) else {
            warn!("couldn't parse message from connection {:?} as typed", connection_entity);
            continue;
        };

        let message_id = u16::from_be_bytes(message_id.try_into().unwrap());

        if message_id != messages.message_id {
            continue;
        }

        // unwrap is safe, contains at least two bytes
        let bytes = bytes.get(2..).unwrap();

        let Ok(message) = bincode::deserialize(bytes) else {
            warn!("couldn't deserialize message from {:?} marked as a \"{}\"", connection_entity, std::any::type_name::<T>());
            continue;
        };

        messages.received.push_back((*connection_entity, message));
    }
}

/// runs just before the sockets update in [UpdateSockets] and serializes typed messages to be sent
fn serialize_typed_messages<T: Serialize + Send + Sync + 'static>(
    mut messages: ResMut<TypedMessages<T>>,
    mut connection_q: Query<&mut Connection>
) {
    for (connection_entity, reliable, message) in messages.send.drain(..) {
        let Ok(mut connection) = connection_q.get_mut(connection_entity) else {
            error!("tried to send a typed message to {:?} but that connection doesn't exist. type was \"{}\"", connection_entity, std::any::type_name::<T>());
            continue;
        };

        connection.send(reliable, message);
    }
}


impl<T> TypedMessages<T> {
    /// returns an iterator of all the messages received this tick
    ///
    /// this method allows multiple systems to read the messages in parallel
    ///
    /// if you need ownership of the message see [take](TypedMessages::take)
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.received.iter().map(|(entity, message)| (*entity, message))
    }

    /// returns an iterator of all the messages received this tick,
    /// but by giving ownership
    ///
    /// this means that for large messages you dont have to copy them,
    /// but only one system can read the messages
    pub fn take(&mut self) -> impl Iterator<Item = (Entity, T)> + '_ {
        self.received.drain(..)
    }

    /// the same as [take](TypedMessages::take) except it will only drain items from [Entity]s specified by a predicate
    pub fn take_from<P: FnMut(Entity) -> bool>(&mut self, predicate: P) -> TakeFromIter<T, P> {
        TakeFromIter {
            predicate,
            position: 0,
            messages: self,
        }
    }

    /// queues a typed message to be sent in the next socket update
    pub fn send(&mut self, connections: Connections, reliable: bool, message: &T) where T: Serialize {

        let Ok(mut message_bytes) = bincode::serialize(message) else {
            error!("failed to serialize typed message \"{}\"", std::any::type_name::<T>());
            return;
        };

        let mut bytes = Vec::from(self.message_id.to_be_bytes());
        bytes.append(&mut message_bytes);

        for entity in connections {
            self.send.push_back((entity, reliable, bytes.clone().into_boxed_slice()));
        }
    }
}


pub struct TakeFromIter<'a, T, P> {
    predicate: P,
    position: usize,
    messages: &'a mut TypedMessages<T>,
}

impl<'a, T, P: FnMut(Entity) -> bool> Iterator for TakeFromIter<'a, T, P> {
    type Item = (Entity, T);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let Some(&(entity, _)) = self.messages.received.get(self.position) else {
                return None;
            };

            if (self.predicate)(entity) {
                return Some(self.messages.received.remove(self.position).unwrap());
            }

            self.position += 1;
        }
    }
}

/// when sending messages use this enum to specify which clients to send a message to
///
/// [Clients] is never stored, so the lifetime parameter allows [Clients::filter] to reference data such as queries
pub enum Connections<'a> {
    None,
    One(Entity),
    Iter(Box<dyn Iterator<Item = Entity> + 'a>),
}

impl<'a> Iterator for Connections<'a> {
    type Item = Entity;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Connections::None => None,
            &mut Connections::One(entity) => {
                *self = Connections::None;
                Some(entity)
            },
            Connections::Iter(iter) => iter.next(),
        }
    }
}

impl<'a> Connections<'a> {
    /// helper function to construct the `Iter` variant
    pub fn iter<I: Iterator<Item = Entity> + 'a>(iter: I) -> Self {
        Connections::Iter(Box::new(iter))
    }
}
