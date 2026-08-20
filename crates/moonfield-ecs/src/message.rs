//! Buffered messages, ported from the reference implementation's
//! `bevy_ecs::message` (0.20; architecture-level, single-threaded).
//!
//! A message is a value sent between systems through a buffered channel:
//! writers push into the [`Messages<M>`] resource (usually via the
//! [`MessageWriter`] system param), readers consume through a per-system
//! [`MessageCursor`] (via the [`MessageReader`] system param), and a
//! once-per-frame update swaps the double buffer, giving every message a
//! two-frame lifetime:
//!
//! - [`Messages::update`] swaps the buffers and clears the oldest one;
//!   a reader that reads at least once per update never drops messages;
//! - a reader that skips an update may still receive some messages;
//! - after two updates without reading, older messages are gone.
//!
//! Register a message type with `App::add_message::<M>()` (moonfield-app),
//! which inserts the resource and wires the buffer swap into the `First`
//! schedule.
//!
//! Minimal-port notes: `Message` is a blanket-implemented marker trait (like
//! `Component`/`Resource` — no derive here); change-tick-based update
//! skipping and the fixed-update signaling of the reference's
//! `message_update_system` are not ported — buffers swap unconditionally once
//! per frame, which has identical observable semantics for per-frame readers.

use std::cell::{Ref, RefMut};
use std::iter::Chain;
use std::marker::PhantomData;
use std::slice::Iter;

use crate::{SystemParam, World};

/// A buffered message payload. Blanket-implemented for every
/// `Send + Sync + 'static` type (like [`Component`](crate::Component)).
pub trait Message: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Message for T {}

/// The unique id of a written message: a monotonically increasing sequence
/// number within its [`Messages<M>`] store.
pub struct MessageId<M: Message> {
    id: usize,
    _marker: PhantomData<M>,
}

impl<M: Message> MessageId<M> {
    /// The message's sequence number.
    pub fn id(&self) -> usize {
        self.id
    }
}

// Manual impls so `MessageId<M>` stays `Copy`/`Eq` regardless of `M`.
impl<M: Message> Copy for MessageId<M> {}
impl<M: Message> Clone for MessageId<M> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<M: Message> PartialEq for MessageId<M> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<M: Message> Eq for MessageId<M> {}
impl<M: Message> std::hash::Hash for MessageId<M> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl<M: Message> std::fmt::Debug for MessageId<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MessageId<{}>({})", std::any::type_name::<M>(), self.id)
    }
}

struct MessageInstance<M> {
    message_id: usize,
    message: M,
}

/// One buffer of the [`Messages`] double buffer, with the sequence number of
/// its first message.
struct MessageSequence<M> {
    messages: Vec<MessageInstance<M>>,
    start_message_count: usize,
}

impl<M> Default for MessageSequence<M> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            start_message_count: 0,
        }
    }
}

/// The message store resource for message type `M`: a double buffer holding
/// the messages of the current and the previous update.
///
/// Inserted automatically by `App::add_message::<M>()`; see the module docs
/// for the retention semantics.
pub struct Messages<M: Message> {
    /// The oldest still-retained messages.
    messages_a: MessageSequence<M>,
    /// The newer messages.
    messages_b: MessageSequence<M>,
    message_count: usize,
}

impl<M: Message> Default for Messages<M> {
    fn default() -> Self {
        Self {
            messages_a: Default::default(),
            messages_b: Default::default(),
            message_count: 0,
        }
    }
}

impl<M: Message> Messages<M> {
    /// Write a message into the current buffer, returning its id.
    pub fn write(&mut self, message: M) -> MessageId<M> {
        let message_id = MessageId {
            id: self.message_count,
            _marker: PhantomData,
        };
        self.messages_b.push_instance(MessageInstance {
            message_id: self.message_count,
            message,
        });
        self.message_count += 1;
        message_id
    }

    /// Write the default value of the message type.
    pub fn write_default(&mut self) -> MessageId<M>
    where
        M: Default,
    {
        self.write(M::default())
    }

    /// A cursor that will read every message currently in the buffers.
    pub fn get_cursor(&self) -> MessageCursor<M> {
        MessageCursor::default()
    }

    /// A cursor that ignores the buffered messages and reads only future
    /// ones.
    pub fn get_cursor_current(&self) -> MessageCursor<M> {
        MessageCursor {
            last_message_count: self.message_count,
            _marker: PhantomData,
        }
    }

    /// Swap the buffers and clear the oldest one. Called once per frame by
    /// [`message_update_system`].
    pub fn update(&mut self) {
        std::mem::swap(&mut self.messages_a, &mut self.messages_b);
        self.messages_b.messages.clear();
        self.messages_b.start_message_count = self.message_count;
        debug_assert_eq!(
            self.messages_a.start_message_count + self.messages_a.messages.len(),
            self.messages_b.start_message_count
        );
    }

    /// The number of messages currently retained (both buffers).
    pub fn len(&self) -> usize {
        self.messages_a.messages.len() + self.messages_b.messages.len()
    }

    /// Whether no messages are currently retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop all retained messages.
    pub fn clear(&mut self) {
        self.messages_a.messages.clear();
        self.messages_a.start_message_count = self.message_count;
        self.messages_b.messages.clear();
        self.messages_b.start_message_count = self.message_count;
    }
}

impl<M> MessageSequence<M> {
    fn push_instance(&mut self, instance: MessageInstance<M>) {
        self.messages.push(instance);
    }
}

/// Per-reader state tracking which messages of a [`Messages<M>`] store have
/// already been seen.
///
/// Usually used indirectly through the [`MessageReader`] system param (which
/// stores it as the system's [`Local`](crate::Local)-style state); it can
/// also be held directly, e.g. by exclusive systems or non-system consumers
/// such as the editor's render loop.
#[derive(Debug)]
pub struct MessageCursor<M: Message> {
    last_message_count: usize,
    _marker: PhantomData<M>,
}

impl<M: Message> Default for MessageCursor<M> {
    fn default() -> Self {
        Self {
            last_message_count: 0,
            _marker: PhantomData,
        }
    }
}

impl<M: Message> Clone for MessageCursor<M> {
    fn clone(&self) -> Self {
        Self {
            last_message_count: self.last_message_count,
            _marker: PhantomData,
        }
    }
}

impl<M: Message> MessageCursor<M> {
    /// Iterate the messages this cursor has not seen yet, oldest first,
    /// advancing the cursor.
    pub fn read<'a>(&'a mut self, messages: &'a Messages<M>) -> MessageIterator<'a, M> {
        self.read_with_id(messages).without_id()
    }

    /// Like [`read`](Self::read), also yielding each message's id.
    pub fn read_with_id<'a>(
        &'a mut self,
        messages: &'a Messages<M>,
    ) -> MessageIteratorWithId<'a, M> {
        MessageIteratorWithId::new(self, messages)
    }

    /// The number of messages not yet seen by this cursor (at most the
    /// number of retained messages; older ones may have been dropped).
    pub fn len(&self, messages: &Messages<M>) -> usize {
        messages
            .message_count
            .saturating_sub(self.last_message_count)
            .min(messages.len())
    }

    /// Whether this cursor has no unread messages.
    pub fn is_empty(&self, messages: &Messages<M>) -> bool {
        self.len(messages) == 0
    }

    /// Mark all current messages as seen without reading them.
    pub fn clear(&mut self, messages: &Messages<M>) {
        self.last_message_count = messages.message_count;
    }
}

/// An iterator over the unread messages of a [`MessageCursor`], advancing
/// the cursor as it yields.
pub struct MessageIterator<'a, M: Message> {
    iter: MessageIteratorWithId<'a, M>,
}

impl<'a, M: Message> Iterator for MessageIterator<'a, M> {
    type Item = &'a M;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(message, _)| message)
    }
}

/// An iterator over the unread messages of a [`MessageCursor`], with ids.
pub struct MessageIteratorWithId<'a, M: Message> {
    reader: &'a mut MessageCursor<M>,
    chain: Chain<Iter<'a, MessageInstance<M>>, Iter<'a, MessageInstance<M>>>,
    unread: usize,
}

impl<'a, M: Message> MessageIteratorWithId<'a, M> {
    fn new(reader: &'a mut MessageCursor<M>, messages: &'a Messages<M>) -> Self {
        let a_index = reader
            .last_message_count
            .saturating_sub(messages.messages_a.start_message_count);
        let b_index = reader
            .last_message_count
            .saturating_sub(messages.messages_b.start_message_count);
        let a = messages
            .messages_a
            .messages
            .get(a_index..)
            .unwrap_or_default();
        let b = messages
            .messages_b
            .messages
            .get(b_index..)
            .unwrap_or_default();

        let unread = a.len() + b.len();
        debug_assert_eq!(unread, reader.len(messages));
        reader.last_message_count = messages.message_count - unread;

        Self {
            reader,
            chain: a.iter().chain(b.iter()),
            unread,
        }
    }

    /// Drop the ids, yielding only the messages.
    pub fn without_id(self) -> MessageIterator<'a, M> {
        MessageIterator { iter: self }
    }
}

impl<'a, M: Message> Iterator for MessageIteratorWithId<'a, M> {
    type Item = (&'a M, MessageId<M>);

    fn next(&mut self) -> Option<Self::Item> {
        let instance = self.chain.next()?;
        self.reader.last_message_count += 1;
        self.unread -= 1;
        Some((
            &instance.message,
            MessageId {
                id: instance.message_id,
                _marker: PhantomData,
            },
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.unread, Some(self.unread))
    }
}

// ---------------------------------------------------------------------
// System params
// ---------------------------------------------------------------------

/// System param reading [`Messages<M>`] in order, tracking per-system which
/// messages have already been seen.
///
/// ```ignore
/// fn react(mut reader: MessageReader<WindowEventKind>) {
///     for event in reader.read() { /* ... */ }
/// }
/// ```
///
/// Panics when fetched if `Messages<M>` is not in the world — register the
/// message type with `App::add_message::<M>()` first.
pub struct MessageReader<'w, 's, M: Message> {
    cursor: &'s mut MessageCursor<M>,
    messages: Ref<'w, Messages<M>>,
}

impl<'w, 's, M: Message> MessageReader<'w, 's, M> {
    /// Iterate the messages this reader has not seen yet, oldest first.
    pub fn read(&mut self) -> MessageIterator<'_, M> {
        self.cursor.read(&self.messages)
    }

    /// Like [`read`](Self::read), also yielding message ids.
    pub fn read_with_id(&mut self) -> MessageIteratorWithId<'_, M> {
        self.cursor.read_with_id(&self.messages)
    }

    /// The number of unread messages for this reader.
    pub fn len(&self) -> usize {
        self.cursor.len(&self.messages)
    }

    /// Whether this reader has no unread messages.
    pub fn is_empty(&self) -> bool {
        self.cursor.is_empty(&self.messages)
    }

    /// Mark all current messages as read without consuming them.
    pub fn clear(&mut self) {
        self.cursor.clear(&self.messages);
    }
}

impl<M: Message> SystemParam for MessageReader<'_, '_, M> {
    type State = MessageCursor<M>;
    type Item<'w, 's> = MessageReader<'w, 's, M>;

    fn init_state() -> Self::State {
        MessageCursor::default()
    }

    fn fetch<'w, 's>(world: &'w World, state: &'s mut Self::State) -> Self::Item<'w, 's> {
        MessageReader {
            cursor: state,
            messages: world.get_resource::<Messages<M>>().unwrap_or_else(|| {
                panic!(
                    "Messages<{}> is not initialized; call `App::add_message` first",
                    std::any::type_name::<M>()
                )
            }),
        }
    }
}

/// System param writing into [`Messages<M>`].
///
/// ```ignore
/// fn emit(mut writer: MessageWriter<AppExit>) {
///     writer.write(AppExit);
/// }
/// ```
///
/// Panics when fetched if `Messages<M>` is not in the world — register the
/// message type with `App::add_message::<M>()` first.
pub struct MessageWriter<'w, M: Message> {
    messages: RefMut<'w, Messages<M>>,
}

impl<M: Message> MessageWriter<'_, M> {
    /// Write a message, returning its id.
    pub fn write(&mut self, message: M) -> MessageId<M> {
        self.messages.write(message)
    }

    /// Write the default value of the message type.
    pub fn write_default(&mut self) -> MessageId<M>
    where
        M: Default,
    {
        self.messages.write_default()
    }
}

impl<M: Message> SystemParam for MessageWriter<'_, M> {
    type State = ();
    type Item<'w, 's> = MessageWriter<'w, M>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        MessageWriter {
            messages: world.get_resource_mut::<Messages<M>>().unwrap_or_else(|| {
                panic!(
                    "Messages<{}> is not initialized; call `App::add_message` first",
                    std::any::type_name::<M>()
                )
            }),
        }
    }
}

// ---------------------------------------------------------------------
// Buffer swapping
// ---------------------------------------------------------------------

/// Resource listing every registered message type's buffer-swap function.
/// Populated by `App::add_message`; consumed by [`message_update_system`].
#[derive(Default)]
pub struct MessageRegistry {
    updates: Vec<fn(&mut World)>,
}

impl MessageRegistry {
    /// Register message type `M` for the once-per-frame buffer swap. The
    /// caller ensures the [`Messages<M>`] resource exists (`App::add_message`
    /// does both).
    pub fn register<M: Message>(&mut self) {
        self.updates.push(|world| {
            if let Some(mut messages) = world.get_resource_mut::<Messages<M>>() {
                messages.update();
            }
        });
    }

    fn update_fns(&self) -> Vec<fn(&mut World)> {
        self.updates.clone()
    }
}

/// Exclusive system swapping the buffers of every registered
/// [`Messages<M>`], giving messages their two-frame lifetime. Runs in the
/// `First` schedule (added there by the first `App::add_message` call).
///
/// Unlike the reference, buffers are swapped unconditionally (our resources
/// carry no per-resource change ticks to skip unchanged stores on); the
/// observable semantics for per-frame readers are identical.
pub fn message_update_system(world: &mut World) {
    let Some(registry) = world.get_resource::<MessageRegistry>() else {
        return;
    };
    let updates = registry.update_fns();
    drop(registry);
    for update in updates {
        update(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoSystemConfigs, ResMut, Schedule};

    #[derive(Debug, PartialEq, Clone, Copy)]
    struct TestMessage(u32);

    fn write_three(messages: &mut Messages<TestMessage>) {
        messages.write(TestMessage(1));
        messages.write(TestMessage(2));
        messages.write(TestMessage(3));
    }

    #[test]
    fn test_write_and_read_once_per_cursor() {
        let mut messages = Messages::<TestMessage>::default();
        write_three(&mut messages);

        let mut cursor = messages.get_cursor();
        let values: Vec<u32> = cursor.read(&messages).map(|m| m.0).collect();
        assert_eq!(values, [1, 2, 3]);
        // A cursor consumes: the second read sees nothing.
        assert_eq!(cursor.read(&messages).count(), 0);
        // A fresh cursor sees everything retained.
        let mut fresh = messages.get_cursor();
        assert_eq!(fresh.read(&messages).count(), 3);
    }

    #[test]
    fn test_two_frame_retention() {
        let mut messages = Messages::<TestMessage>::default();
        messages.write(TestMessage(1));
        messages.update();
        // Written before the swap: still readable (now in the old buffer).
        messages.write(TestMessage(2));
        let mut all = messages.get_cursor();
        let values: Vec<u32> = all.read(&messages).map(|m| m.0).collect();
        assert_eq!(values, [1, 2]);

        // After two swaps the first message is gone.
        messages.update();
        let mut all = messages.get_cursor();
        let values: Vec<u32> = all.read(&messages).map(|m| m.0).collect();
        assert_eq!(values, [2]);
        messages.update();
        let mut all = messages.get_cursor();
        assert_eq!(all.read(&messages).count(), 0);
    }

    #[test]
    fn test_reader_that_skips_updates_drops_old_messages() {
        let mut messages = Messages::<TestMessage>::default();
        let mut cursor = messages.get_cursor();
        messages.write(TestMessage(1));
        messages.update();
        messages.write(TestMessage(2));
        messages.update();
        // Message 1 has aged out; the cursor sees only message 2.
        let values: Vec<u32> = cursor.read(&messages).map(|m| m.0).collect();
        assert_eq!(values, [2]);
    }

    #[test]
    fn test_len_is_empty_and_clear() {
        let mut messages = Messages::<TestMessage>::default();
        let mut cursor = messages.get_cursor_current();
        assert!(cursor.is_empty(&messages));
        write_three(&mut messages);
        assert_eq!(cursor.len(&messages), 3);
        cursor.clear(&messages);
        assert!(cursor.is_empty(&messages));
        assert_eq!(cursor.read(&messages).count(), 0);
        assert_eq!(messages.len(), 3);
        messages.clear();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_read_with_id_reports_sequence_numbers() {
        let mut messages = Messages::<TestMessage>::default();
        write_three(&mut messages);
        let mut cursor = messages.get_cursor();
        let ids: Vec<usize> = cursor
            .read_with_id(&messages)
            .map(|(_, id)| id.id())
            .collect();
        assert_eq!(ids, [0, 1, 2]);
    }

    #[test]
    fn test_reader_writer_params_across_frames() {
        #[derive(Default)]
        struct Sum(u32);

        fn writer(mut out: MessageWriter<TestMessage>, mut sum: ResMut<Sum>) {
            sum.0 += 1;
            out.write(TestMessage(sum.0));
        }
        fn reader(mut input: MessageReader<TestMessage>, mut total: ResMut<Total>) {
            for message in input.read() {
                total.0 += message.0;
            }
        }
        #[derive(Default)]
        struct Total(u32);

        let mut world = World::new();
        world.insert_resource(Sum::default());
        world.insert_resource(Total::default());
        world.insert_resource(Messages::<TestMessage>::default());
        world.insert_resource(MessageRegistry::default());
        world
            .get_resource_mut::<MessageRegistry>()
            .unwrap()
            .register::<TestMessage>();

        let mut schedule = Schedule::new();
        schedule.add_systems((writer, reader.after(&writer)));

        // Frame 1: write 1, read it.
        message_update_system(&mut world);
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Total>().unwrap().0, 1);
        // Frame 2: write 2, read only the new one.
        message_update_system(&mut world);
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Total>().unwrap().0, 3);
        // A second reader system would track its own cursor independently —
        // covered by per-cursor tests above.
    }
}
