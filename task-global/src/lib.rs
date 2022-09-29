//! Have you ever thought *"Dang! I sure would love to use global variables in
//! my asynchronous tasks, but with all the thread-hopping and
//! cooperative-multi-tasking that'd be impossible"*? Well lament no more!
//! Because now you can...
//!
//! # Task Globals
//!
//! The [`TaskGlobal`] trait allows you to add "global" variables to your tasks
//! and access them from anywhere within the task:
//!
//! ```
//! use task_global::{TaskGlobal, TaskGlobalExt};
//!
//! #[derive(Default, TaskGlobal)]
//! struct Context {
//!     value: i32,
//! }
//!
//! # async fn f() {
//! async {
//!     // set the global
//!     Context::global_mut(|ctx| ctx.value = 42);
//!
//!     // get the global
//!     let value = Context::global(|ctx| ctx.value);
//!     println!("{}", value); // prints 42
//!
//! }.with_global(Context::default()).await;
//! # }
//! ```
//!
//! ## Scoping
//!
//! The "global" value is only available while the annotated task is executing.
//! It does not persist after the task has completed. It is not accessible while
//! the task is idle. It is not available from tasks "spawned" from the
//! annotated task. Annotated tasks can be nested and the value will be the one
//! set from the most-recently set scope.
//!
//! ```
//! # use task_global::{TaskGlobal, TaskGlobalExt};
//! # #[derive(Default, TaskGlobal)]
//! # struct Context { value: i32 }
//! # async fn f() {
//! Context::try_global(|ctx| assert!(ctx.is_none()));
//!
//! async {
//!     Context::global(|ctx| assert!(ctx.value == 0));
//!     Context::global_mut(|ctx| ctx.value = 42);
//!     Context::global(|ctx| assert!(ctx.value == 42));
//!
//!     async {
//!         Context::global(|ctx| assert!(ctx.value == 0));
//!         Context::global_mut(|ctx| ctx.value = 5);
//!         Context::global(|ctx| assert!(ctx.value == 5));
//!
//!     }.with_global(Context::default()).await;
//!
//!     Context::global(|ctx| assert!(ctx.value == 42));
//!
//! }.with_global(Context::default()).await;
//!
//! Context::try_global(|ctx| assert!(ctx.is_none()));
//! # }
//! ```
//!
//! ## How does it work?
//!
//! While [`Future`][std::future::Future]s are often moved between threads and their progress may be
//! interleaved with others, they cannot do so while they are executing. While
//! `.poll()` is running, nothing else can happen on that thread and the
//! `Future` cannot be moved or destroyed. This takes advantage of that by
//! storing a handle to the "global" value in thread-local storage that is
//! accessible by the free methods. It puts the handle in when `.poll()` starts,
//! and it takes it back out before it ends.
//!
//! That could be the end of it, except nested uses of the annotated task would
//! stomp on the thread-local storage of their parent annotated tasks. This
//! avoids that problem by essentially storing a linked-list of handles, that
//! can be pushed and popped when tasks are suspended and resumed. Doing this
//! also allows access to prior scopes via the [`global_chain`][TaskGlobal::global_chain] methods, which
//! could be useful in certain situations.
//!
//! ## Should I actually use this?
//!
//! If you have the need for it.
//!
//! You should follow the same general advice for global variables and avoid
//! them if there is an alternative. However, sometimes using a global variable
//! is ergonomically advantageous, and this gives you that option in an
//! asynchronous context.

use std::cell::{Ref, RefCell, RefMut};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::LocalKey;

use pin_project::pin_project;

/// This takes care of implementing [`TaskGlobal`] by setting up storage and
/// using it for the required [`key()`][TaskGlobal::key] method.
///
/// This derive macro does not use any attributes.
///
/// Any `#[derive]`-able types are allowed. Generics are not allowed (neither
/// lifetimes nor types). If you require storing a type that is generic with
/// specific generic parameters, you must implement it manually.
#[cfg(feature = "macros")]
pub use task_global_macros::TaskGlobal;

/// Implementing this trait allows the type to be used with [`TaskGlobalExt::with_global`] on
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
/// with the current global type while in one of these calls will also panic.
/// Implementations of `TaskGlobal` on different types do not interfere with
/// each other.
pub trait TaskGlobal: Sized + 'static {
    /// This returns a reference to the thread-local key that will be used to
    /// store "global" values of this type. This is auto-implemented by the
    /// derive macro, but if implemented manually should look something like
    /// this:
    ///
    /// ```
    /// use std::thread::LocalKey;
    /// use task_global::Storage;
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

    /// This method attempts to call the function with the currently stored
    /// value but will pass `None` if outside the execution of an annotated
    /// task.
    fn try_global<F, R>(f: F) -> R
    where
        F: FnOnce(Option<&Self>) -> R,
    {
        Self::key().with(|storage| f(storage.current().as_deref()))
    }

    /// This method attempts to call the function with the currently stored
    /// value as a mutable reference but will pass `None` if outside the
    /// execution of an annotated task.
    fn try_global_mut<F, R>(f: F) -> R
    where
        F: FnOnce(Option<&mut Self>) -> R,
    {
        Self::key().with(|storage| f(storage.current_mut().as_deref_mut()))
    }

    /// This method will call the function with a reference to the currently
    /// stored value. It will panic if outside the execution of an annotated
    /// task.
    fn global<F, R>(f: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        Self::try_global(|maybe_current| {
            f(maybe_current.expect("no value stored in task global storage"))
        })
    }

    /// This method will call the function with a mutable reference to the
    /// currently stored value. It will panic if outside the execution of an
    /// annotated task.
    fn global_mut<F, R>(f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        Self::try_global_mut(|maybe_current| {
            f(maybe_current.expect("no value stored in task global storage"))
        })
    }

    /// This method will call the function with an iterator yielding references
    /// to all the currently stored values, in most-recently-annotated order.
    fn global_chain<F, R>(f: F) -> R
    where
        F: FnOnce(StorageIter<'_, Self>) -> R,
    {
        Self::key().with(|storage| f(StorageIter::new(&storage.head())))
    }

    /// This method will call the function with an iterator yielding mutable
    /// references to all the currently stored values, in
    /// most-recently-annotated order.
    fn global_chain_mut<F, R>(f: F) -> R
    where
        F: FnOnce(StorageIterMut<'_, Self>) -> R,
    {
        Self::key().with(|storage| f(StorageIterMut::new(&mut storage.head_mut())))
    }
}

/// An extension type for [`Future`]s to annotate them with a [`TaskGlobal`]
/// type.
pub trait TaskGlobalExt {
    /// This will construct a [`Future`] type that stores this value and will
    /// make it available to the [`TaskGlobal`] methods when executing.
    fn with_global<T: TaskGlobal>(self, value: T) -> TaskGlobalFuture<Self, T>
    where
        Self: Sized,
    {
        TaskGlobalFuture::new(self, value)
    }
}

impl<Fut> TaskGlobalExt for Fut where Fut: Future {}

/// An iterator that yields references to all the currently accessible "global"
/// values.
pub struct StorageIter<'a, T> {
    current: Option<&'a TaskGlobalNode<T>>,
}

impl<'a, T> StorageIter<'a, T> {
    fn new(head: &'a Option<Box<TaskGlobalNode<T>>>) -> StorageIter<'a, T> {
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
/// "global" values.
pub struct StorageIterMut<'a, T> {
    current: Option<&'a mut TaskGlobalNode<T>>,
}

impl<'a, T> StorageIterMut<'a, T> {
    fn new(head: &'a mut Option<Box<TaskGlobalNode<T>>>) -> StorageIterMut<'a, T> {
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

/// A [`Future`] type that wraps another [`Future`] and stores a "global" value
/// which will be available to the [`TaskGlobal`] methods when executing.
#[pin_project]
pub struct TaskGlobalFuture<Fut, T> {
    #[pin]
    inner: Fut,
    value: Option<Box<TaskGlobalNode<T>>>,
}

impl<Fut, T> TaskGlobalFuture<Fut, T> {
    /// Constructs the type with the given [`Future`] and "global" value.
    pub fn new(inner: Fut, value: T) -> TaskGlobalFuture<Fut, T> {
        TaskGlobalFuture {
            inner,
            value: Some(Box::new(TaskGlobalNode::new(value))),
        }
    }
}

impl<Fut, T> Future for TaskGlobalFuture<Fut, T>
where
    Fut: Future,
    T: TaskGlobal + 'static,
{
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = TaskGlobalNodeGuard::new(T::key(), this.value);
        this.inner.poll(cx)
    }
}

/// An opaque type used to hold and facilitate access to the task "global"
/// variables.
///
/// This type should only be used when implementing [`TaskGlobal::key`]
/// manually.
pub struct Storage<T> {
    head: RefCell<Option<Box<TaskGlobalNode<T>>>>,
}

impl<T> Storage<T> {
    fn current(&self) -> Option<Ref<'_, T>> {
        Ref::filter_map(self.head.borrow(), |head| {
            head.as_ref().map(|node| &node.value)
        })
        .ok()
    }

    fn current_mut(&self) -> Option<RefMut<'_, T>> {
        RefMut::filter_map(self.head.borrow_mut(), |head| {
            head.as_mut().map(|node| &mut node.value)
        })
        .ok()
    }

    fn head(&self) -> Ref<'_, Option<Box<TaskGlobalNode<T>>>> {
        self.head.borrow()
    }

    fn head_mut(&self) -> RefMut<'_, Option<Box<TaskGlobalNode<T>>>> {
        self.head.borrow_mut()
    }

    fn push(&self, mut node: Box<TaskGlobalNode<T>>) {
        let mut head = self.head.borrow_mut();
        node.parent = head.take();
        *head = Some(node);
    }

    fn pop(&self) -> Option<Box<TaskGlobalNode<T>>> {
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

struct TaskGlobalNode<T> {
    value: T,
    parent: Option<Box<TaskGlobalNode<T>>>,
}

impl<T> TaskGlobalNode<T> {
    fn new(value: T) -> TaskGlobalNode<T> {
        TaskGlobalNode {
            value,
            parent: None,
        }
    }

    fn as_parts(&self) -> (&T, &Option<Box<TaskGlobalNode<T>>>) {
        (&self.value, &self.parent)
    }

    fn as_parts_mut(&mut self) -> (&mut T, &mut Option<Box<TaskGlobalNode<T>>>) {
        (&mut self.value, &mut self.parent)
    }
}

struct TaskGlobalNodeGuard<'a, T: 'static> {
    key: &'static LocalKey<Storage<T>>,
    current: &'a mut Option<Box<TaskGlobalNode<T>>>,
}

impl<'a, T: 'static> TaskGlobalNodeGuard<'a, T> {
    fn new(
        key: &'static LocalKey<Storage<T>>,
        current: &'a mut Option<Box<TaskGlobalNode<T>>>,
    ) -> TaskGlobalNodeGuard<'a, T> {
        key.with(|storage| storage.push(current.take().unwrap()));
        TaskGlobalNodeGuard { key, current }
    }
}

impl<'a, T> Drop for TaskGlobalNodeGuard<'a, T> {
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

        storage.push(Box::new(TaskGlobalNode::new(1)));
        storage.push(Box::new(TaskGlobalNode::new(2)));
        storage.push(Box::new(TaskGlobalNode::new(3)));

        assert!(matches!(
            storage.pop().as_deref(),
            Some(TaskGlobalNode {
                value: 3,
                parent: None
            })
        ));
        assert!(matches!(
            storage.pop().as_deref(),
            Some(TaskGlobalNode {
                value: 2,
                parent: None
            })
        ));
        assert!(matches!(
            storage.pop().as_deref(),
            Some(TaskGlobalNode {
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
        impl TaskGlobal for Context {
            fn key() -> &'static LocalKey<Storage<Self>> {
                &STORAGE
            }
        }

        assert!(STORAGE.with(|storage| storage.head.borrow().is_none()));
        TaskGlobalFuture::new(
            async {
                assert!(STORAGE.with(|storage| storage.head.borrow().is_some()));
            },
            Context,
        )
        .await;
        assert!(STORAGE.with(|storage| storage.head.borrow().is_none()));
    }

    #[tokio::test]
    async fn task_global_trait_accesses_value() {
        struct Context;
        impl TaskGlobal for Context {
            fn key() -> &'static LocalKey<Storage<Self>> {
                thread_local!(static STORAGE: Storage<Context> = Storage::default());
                &STORAGE
            }
        }

        assert!(Context::try_global(|context| context.is_none()));
        assert!(Context::try_global_mut(|context| context.is_none()));
        assert!(Context::global_chain(|mut c| c.next().is_none()));
        assert!(Context::global_chain_mut(|mut c| c.next().is_none()));
        TaskGlobalFuture::new(
            async {
                assert!(Context::try_global(|context| context.is_some()));
                assert!(Context::try_global_mut(|context| context.is_some()));
                assert!(Context::global(|context| matches!(context, Context)));
                assert!(Context::global_mut(|context| matches!(context, Context)));
                assert!(Context::global_chain(|mut c| {
                    c.next().is_some() && c.next().is_none()
                }));
                assert!(Context::global_chain_mut(
                    |mut c| c.next().is_some() && c.next().is_none()
                ));
            },
            Context,
        )
        .await;
        assert!(Context::try_global(|context| context.is_none()));
        assert!(Context::try_global_mut(|context| context.is_none()));
        assert!(Context::global_chain(|mut c| c.next().is_none()));
        assert!(Context::global_chain_mut(|mut c| c.next().is_none()));
    }

    #[tokio::test]
    async fn ext_trait_works() {
        struct Context;
        impl TaskGlobal for Context {
            fn key() -> &'static LocalKey<Storage<Self>> {
                thread_local!(static STORAGE: Storage<Context> = Storage::default());
                &STORAGE
            }
        }

        assert!(Context::try_global(|context| context.is_none()));
        async {
            assert!(Context::global(|context| matches!(context, Context)));
        }
        .with_global(Context)
        .await;
        assert!(Context::try_global(|context| context.is_none()));
    }
}
