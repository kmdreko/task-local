//! Have you ever thought *"Dang! I sure would love to use global or thread-local
//! variables in my asynchronous tasks, but with all the thread-hopping and
//! cooperative-multi-tasking that'd be impossible"*? Well lament no more!
//! Because now you can...
//!
//! # Task Locals
//!
//! The [`TaskLocal`] trait allows you to add "local" variables to your tasks
//! and access them from anywhere within the task:
//!
//! ```
//! use task_local::{TaskLocal, TaskLocalExt};
//!
//! #[derive(Default, TaskLocal)]
//! struct Context {
//!     value: i32,
//! }
//!
//! # async fn f() {
//! async {
//!     // set the local
//!     Context::local_mut(|ctx| ctx.value = 42);
//!
//!     // get the local
//!     let value = Context::local(|ctx| ctx.value);
//!     println!("{}", value); // prints 42
//!
//! }.with_local(Context::default()).await;
//! # }
//! ```
//!
//! ## Scoping
//!
//! The local value is only available while the annotated task is executing.
//! It does not persist after the task has completed. It is not accessible while
//! the task is idle. It is not available from tasks "spawned" from the
//! annotated task. Annotated tasks can be nested and the value will be the one
//! set from the most-recently set scope.
//!
//! ```
//! # use task_local::{TaskLocal, TaskLocalExt};
//! # #[derive(Default, TaskLocal)]
//! # struct Context { value: i32 }
//! # async fn f() {
//! assert!(Context::try_local(|_ctx| {}).is_err());
//!
//! async {
//!     Context::local(|ctx| assert!(ctx.value == 0));
//!     Context::local_mut(|ctx| ctx.value = 42);
//!     Context::local(|ctx| assert!(ctx.value == 42));
//!
//!     async {
//!         Context::local(|ctx| assert!(ctx.value == 0));
//!         Context::local_mut(|ctx| ctx.value = 5);
//!         Context::local(|ctx| assert!(ctx.value == 5));
//!
//!     }.with_local(Context::default()).await;
//!
//!     Context::local(|ctx| assert!(ctx.value == 42));
//!
//! }.with_local(Context::default()).await;
//!
//! assert!(Context::try_local(|_ctx| {}).is_err());
//! # }
//! ```
//!
//! ## How does it work?
//!
//! While [`Future`][std::future::Future]s are often moved between threads and their progress may be
//! interleaved with others, they cannot do so while they are executing. While
//! `.poll()` is running, nothing else can happen on that thread and the
//! `Future` cannot be moved or destroyed. This takes advantage of that by
//! storing a handle to the task-local value in thread-local storage that is
//! accessible by the free methods. It puts the handle in when `.poll()` starts,
//! and it takes it back out before it ends.
//!
//! That could be the end of it, except nested uses of the annotated task would
//! stomp on the thread-local storage of their parent annotated tasks. This
//! avoids that problem by essentially storing a linked-list of handles, that
//! can be pushed and popped when tasks are suspended and resumed. Doing this
//! also allows access to prior scopes via the [`local_chain`][TaskLocal::local_chain] methods, which
//! could be useful in certain situations.
//!
//! ## Should I actually use this?
//!
//! If you have the need for it.
//!
//! You should follow the same general advice for global and thread-local
//! variables and avoid them if there is an alternative. However, sometimes
//! using a local variable is ergonomically advantageous, and this gives you
//! that option in an asynchronous context.

use std::cell::{Ref, RefCell, RefMut};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::LocalKey;

use pin_project::pin_project;

/// This takes care of implementing [`TaskLocal`] by setting up storage and
/// using it for the required [`key()`][TaskLocal::key] method.
///
/// This derive macro does not use any attributes.
///
/// Any `#[derive]`-able types are allowed. Generics are not allowed (neither
/// lifetimes nor types). If you require storing a type that is generic with
/// specific generic parameters, you must implement it manually.
#[cfg(feature = "macros")]
pub use task_local_macros::TaskLocal;

/// Implementing this trait allows the type to be used with [`TaskLocalExt::with_local`] on
/// asynchronous tasks and be accessible via the other associated methods while
/// executing those tasks.
///
/// All of the access methods work with function parameters instead of returning
/// references because it can't return a `'static` lifetime, there is no object
/// available to bind the lifetime to, and any references can't be held across
/// `.await`s.
///
/// You should ensure care that `*_mut` are not called while within the call of
/// another method. It will panic if that happens. This is to ensure Rust's
/// normal referential guarantees. Attempting to execute a nested task annotated
/// with the current local type while in one of these calls will also panic.
/// Implementations of `TaskLocal` on different types do not interfere with
/// each other.
pub trait TaskLocal: Sized + 'static {
    /// This returns a reference to the thread-local key that will be used to
    /// store task-local values of this type. This is auto-implemented by the
    /// derive macro, but if implemented manually should look something like
    /// this:
    ///
    /// ```
    /// use std::thread::LocalKey;
    /// use task_local::Storage;
    ///
    /// # struct Context;
    /// # impl Context {
    /// fn key() -> &'static LocalKey<Storage<Context>> {
    ///     thread_local!(static STORAGE: Storage<Context> = Storage::default());
    ///     &STORAGE
    /// }
    /// # }
    /// ```
    fn key() -> &'static LocalKey<Storage<Self>>;

    /// This method attempts to call the function with a reference to the
    /// currently stored value or return an error if it cannot be accessed.
    fn try_local<F, R>(f: F) -> Result<R, StorageError>
    where
        F: FnOnce(&Self) -> R,
    {
        Self::key().with(|storage| storage.current().map(|value| f(&value)))
    }

    /// This method attempts to call the function with a mutable reference to
    /// the currently stored value or return an error if it cannot be accessed.
    fn try_local_mut<F, R>(f: F) -> Result<R, StorageError>
    where
        F: FnOnce(&mut Self) -> R,
    {
        Self::key().with(|storage| storage.current_mut().map(|mut value| f(&mut value)))
    }

    /// This method will call the function with a reference to the currently
    /// stored value. It will panic if outside the execution of an annotated
    /// task.
    fn local<F, R>(f: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        Self::try_local(f).expect("cannot access the value in task local storage")
    }

    /// This method will call the function with a mutable reference to the
    /// currently stored value. It will panic if outside the execution of an
    /// annotated task.
    fn local_mut<F, R>(f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        Self::try_local_mut(f).expect("cannot access the value in task local storage")
    }

    /// This method attempts to call the function with an iterator yielding
    /// references to all the currently stored values, in most-recently-annotated
    /// order or will return an error if it cannot be accessed. This will not
    /// return `StorageError::NoValue`, the iterator will simply be empty.
    fn try_local_chain<F, R>(f: F) -> Result<R, StorageError>
    where
        F: FnOnce(StorageIter<'_, Self>) -> R,
    {
        Self::key().with(|storage| storage.head().map(|head| f(StorageIter::new(&head))))
    }

    /// This method attempts to call the function with an iterator yielding
    /// mutable references to all the currently stored values, in
    /// most-recently-annotated order or will return an error if it cannot be
    /// accessed. This will not return `StorageError::NoValue`, the iterator
    /// will simply be empty.
    fn try_local_chain_mut<F, R>(f: F) -> Result<R, StorageError>
    where
        F: FnOnce(StorageIterMut<'_, Self>) -> R,
    {
        Self::key().with(|storage| {
            storage
                .head_mut()
                .map(|mut head| f(StorageIterMut::new(&mut head)))
        })
    }

    /// This method will call the function with an iterator yielding references
    /// to all the currently stored values, in most-recently-annotated order.
    fn local_chain<F, R>(f: F) -> R
    where
        F: FnOnce(StorageIter<'_, Self>) -> R,
    {
        Self::try_local_chain(f).expect("cannot access task local storage")
    }

    /// This method will call the function with an iterator yielding mutable
    /// references to all the currently stored values, in
    /// most-recently-annotated order.
    fn local_chain_mut<F, R>(f: F) -> R
    where
        F: FnOnce(StorageIterMut<'_, Self>) -> R,
    {
        Self::try_local_chain_mut(f).expect("cannot access task local storage")
    }
}

/// An extension type for [`Future`]s to annotate them with a [`TaskLocal`]
/// type.
pub trait TaskLocalExt {
    /// This will construct a [`Future`] type that stores this value and will
    /// make it available to the [`TaskLocal`] methods when executing.
    fn with_local<T: TaskLocal>(self, value: T) -> TaskLocalFuture<Self, T>
    where
        Self: Sized,
    {
        TaskLocalFuture::new(self, value)
    }
}

impl<Fut> TaskLocalExt for Fut where Fut: Future {}

/// An error that is returned from the `TaskLocal::try_*` methods if the
/// local value cannot be accessed.
#[derive(Copy, Clone, Debug)]
pub enum StorageError {
    /// There is no value in storage. This means that you are not in a thread of
    /// execution that is not wrapped in a `TaskLocalFuture`.
    NoValue,

    /// The storage cannot be accessed because it is currently locked. This can
    /// happen if you call the free methods recursively and at least one of them
    /// is accessing the storage mutably.
    Locked,
}

/// An iterator that yields references to all the currently accessible local
/// values.
pub struct StorageIter<'a, T> {
    current: Option<&'a TaskLocalNode<T>>,
}

impl<'a, T> StorageIter<'a, T> {
    fn new(head: &'a Option<Box<TaskLocalNode<T>>>) -> StorageIter<'a, T> {
        StorageIter {
            current: head.as_deref(),
        }
    }
}

impl<'a, T> Iterator for StorageIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.current.take().map(|current| {
            let (current, next) = current.as_parts();
            self.current = next.as_deref();
            current
        })
    }
}

/// An iterator that yields mutable references to all the currently accessible
/// local values.
pub struct StorageIterMut<'a, T> {
    current: Option<&'a mut TaskLocalNode<T>>,
}

impl<'a, T> StorageIterMut<'a, T> {
    fn new(head: &'a mut Option<Box<TaskLocalNode<T>>>) -> StorageIterMut<'a, T> {
        StorageIterMut {
            current: head.as_deref_mut(),
        }
    }
}

impl<'a, T> Iterator for StorageIterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.current.take().map(|current| {
            let (current, next) = current.as_parts_mut();
            self.current = next.as_deref_mut();
            current
        })
    }
}

/// A [`Future`] type that wraps another [`Future`] and stores a local value
/// which will be available to the [`TaskLocal`] methods when executing.
#[pin_project]
pub struct TaskLocalFuture<Fut, T> {
    #[pin]
    inner: Fut,
    value: Option<Box<TaskLocalNode<T>>>,
}

impl<Fut, T> TaskLocalFuture<Fut, T> {
    /// Constructs the type with the given [`Future`] and local value.
    pub fn new(inner: Fut, value: T) -> TaskLocalFuture<Fut, T> {
        TaskLocalFuture {
            inner,
            value: Some(Box::new(TaskLocalNode::new(value))),
        }
    }
}

impl<Fut, T> Future for TaskLocalFuture<Fut, T>
where
    Fut: Future,
    T: TaskLocal,
{
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = TaskLocalNodeGuard::new(T::key(), this.value);
        this.inner.poll(cx)
    }
}

#[cfg(feature = "futures")]
impl<Fut, T> futures::future::FusedFuture for TaskLocalFuture<Fut, T>
where
    Fut: futures::future::FusedFuture,
    T: TaskLocal,
{
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

/// An opaque type used to hold and facilitate access to the task-local values.
///
/// This type should only be used when implementing [`TaskLocal::key`]
/// manually.
pub struct Storage<T> {
    head: RefCell<Option<Box<TaskLocalNode<T>>>>,
}

impl<T> Storage<T> {
    fn current(&self) -> Result<Ref<'_, T>, StorageError> {
        use StorageError::*;

        let head = self.head.try_borrow().map_err(|_| Locked)?;
        let value = Ref::filter_map(head, |head| head.as_ref().map(|node| &node.value))
            .map_err(|_| NoValue)?;

        Ok(value)
    }

    fn current_mut(&self) -> Result<RefMut<'_, T>, StorageError> {
        use StorageError::*;

        let head = self.head.try_borrow_mut().map_err(|_| Locked)?;
        let value = RefMut::filter_map(head, |head| head.as_mut().map(|node| &mut node.value))
            .map_err(|_| NoValue)?;

        Ok(value)
    }

    fn head(&self) -> Result<Ref<'_, Option<Box<TaskLocalNode<T>>>>, StorageError> {
        self.head.try_borrow().map_err(|_| StorageError::Locked)
    }

    fn head_mut(&self) -> Result<RefMut<'_, Option<Box<TaskLocalNode<T>>>>, StorageError> {
        self.head.try_borrow_mut().map_err(|_| StorageError::Locked)
    }

    fn push(&self, mut node: Box<TaskLocalNode<T>>) {
        let mut head = self.head.borrow_mut();
        node.parent = head.take();
        *head = Some(node);
    }

    fn pop(&self) -> Option<Box<TaskLocalNode<T>>> {
        let mut head = self.head.borrow_mut();
        let mut node = head.take();
        *head = node.as_mut().and_then(|node| node.parent.take());
        node
    }
}

impl<T> Default for Storage<T> {
    fn default() -> Self {
        Self {
            head: RefCell::new(None),
        }
    }
}

struct TaskLocalNode<T> {
    value: T,
    parent: Option<Box<TaskLocalNode<T>>>,
}

impl<T> TaskLocalNode<T> {
    fn new(value: T) -> TaskLocalNode<T> {
        TaskLocalNode {
            value,
            parent: None,
        }
    }

    fn as_parts(&self) -> (&T, &Option<Box<TaskLocalNode<T>>>) {
        (&self.value, &self.parent)
    }

    fn as_parts_mut(&mut self) -> (&mut T, &mut Option<Box<TaskLocalNode<T>>>) {
        (&mut self.value, &mut self.parent)
    }
}

struct TaskLocalNodeGuard<'a, T: 'static> {
    key: &'static LocalKey<Storage<T>>,
    current: &'a mut Option<Box<TaskLocalNode<T>>>,
}

impl<'a, T: 'static> TaskLocalNodeGuard<'a, T> {
    fn new(
        key: &'static LocalKey<Storage<T>>,
        current: &'a mut Option<Box<TaskLocalNode<T>>>,
    ) -> TaskLocalNodeGuard<'a, T> {
        key.with(|storage| storage.push(current.take().unwrap()));
        TaskLocalNodeGuard { key, current }
    }
}

impl<'a, T> Drop for TaskLocalNodeGuard<'a, T> {
    fn drop(&mut self) {
        *self.current = Some(self.key.with(|storage| storage.pop().unwrap()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_can_push_and_pop() {
        let storage = Storage::default();

        storage.push(Box::new(TaskLocalNode::new(1)));
        storage.push(Box::new(TaskLocalNode::new(2)));
        storage.push(Box::new(TaskLocalNode::new(3)));

        assert!(matches!(
            storage.pop().as_deref(),
            Some(TaskLocalNode {
                value: 3,
                parent: None
            })
        ));
        assert!(matches!(
            storage.pop().as_deref(),
            Some(TaskLocalNode {
                value: 2,
                parent: None
            })
        ));
        assert!(matches!(
            storage.pop().as_deref(),
            Some(TaskLocalNode {
                value: 1,
                parent: None
            })
        ));
        assert!(matches!(storage.pop(), None));
    }

    #[tokio::test]
    async fn future_enables_storage() {
        thread_local!(static STORAGE: Storage<Context> = Storage::default());
        struct Context;
        impl TaskLocal for Context {
            fn key() -> &'static LocalKey<Storage<Self>> {
                &STORAGE
            }
        }

        assert!(STORAGE.with(|storage| storage.head.borrow().is_none()));
        TaskLocalFuture::new(
            async {
                assert!(STORAGE.with(|storage| storage.head.borrow().is_some()));
            },
            Context,
        )
        .await;
        assert!(STORAGE.with(|storage| storage.head.borrow().is_none()));
    }

    #[tokio::test]
    async fn task_local_trait_accesses_value() {
        struct Context;
        impl TaskLocal for Context {
            fn key() -> &'static LocalKey<Storage<Self>> {
                thread_local!(static STORAGE: Storage<Context> = Storage::default());
                &STORAGE
            }
        }

        assert!(Context::try_local(|_| {}).is_err());
        assert!(Context::try_local_mut(|_| {}).is_err());
        assert!(Context::local_chain(|mut c| c.next().is_none()));
        assert!(Context::local_chain_mut(|mut c| c.next().is_none()));
        TaskLocalFuture::new(
            async {
                assert!(Context::try_local(|_| {}).is_ok());
                assert!(Context::try_local_mut(|_| {}).is_ok());
                assert!(Context::local(|context| matches!(context, Context)));
                assert!(Context::local_mut(|context| matches!(context, Context)));
                assert!(Context::local_chain(|mut c| {
                    c.next().is_some() && c.next().is_none()
                }));
                assert!(Context::local_chain_mut(
                    |mut c| c.next().is_some() && c.next().is_none()
                ));
            },
            Context,
        )
        .await;
        assert!(Context::try_local(|_| {}).is_err());
        assert!(Context::try_local_mut(|_| {}).is_err());
        assert!(Context::local_chain(|mut c| c.next().is_none()));
        assert!(Context::local_chain_mut(|mut c| c.next().is_none()));
    }

    #[tokio::test]
    async fn ext_trait_works() {
        struct Context;
        impl TaskLocal for Context {
            fn key() -> &'static LocalKey<Storage<Self>> {
                thread_local!(static STORAGE: Storage<Context> = Storage::default());
                &STORAGE
            }
        }

        assert!(Context::try_local(|_| {}).is_err());
        async {
            assert!(Context::local(|context| matches!(context, Context)));
        }
        .with_local(Context)
        .await;
        assert!(Context::try_local(|_| {}).is_err());
    }
}
