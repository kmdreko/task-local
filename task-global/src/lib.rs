//! Have you ever thought *"Dang! I sure would love to use global variables in my asynchronous tasks, but with all the thread-hopping and cooperative-multi-tasking that'd be impossible"*?

use std::cell::{Ref, RefCell, RefMut};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::LocalKey;

use pin_project::pin_project;

#[cfg(feature = "macros")]
pub use task_global_macros::TaskGlobal;

pub trait TaskGlobal: Sized + 'static {
    fn key() -> &'static LocalKey<Storage<Self>>;

    fn try_global<F, R>(f: F) -> R
    where
        F: FnOnce(Option<&Self>) -> R,
    {
        Self::key().with(|storage| f(storage.current().as_deref()))
    }

    fn try_global_mut<F, R>(f: F) -> R
    where
        F: FnOnce(Option<&mut Self>) -> R,
    {
        Self::key().with(|storage| f(storage.current_mut().as_deref_mut()))
    }

    fn global<F, R>(f: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        Self::try_global(|maybe_current| {
            f(maybe_current.expect("no value stored in task global storage"))
        })
    }

    fn global_mut<F, R>(f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        Self::try_global_mut(|maybe_current| {
            f(maybe_current.expect("no value stored in task global storage"))
        })
    }

    fn global_chain<F, R>(f: F) -> R
    where
        F: FnOnce(StorageIter<'_, Self>) -> R,
    {
        Self::key().with(|storage| f(StorageIter::new(&storage.head())))
    }

    fn global_chain_mut<F, R>(f: F) -> R
    where
        F: FnOnce(StorageIterMut<'_, Self>) -> R,
    {
        Self::key().with(|storage| f(StorageIterMut::new(&mut storage.head_mut())))
    }
}

pub trait TaskGlobalExt {
    fn with_global<T: TaskGlobal>(self, value: T) -> TaskGlobalFuture<Self, T>
    where
        Self: Sized,
    {
        TaskGlobalFuture::new(self, value)
    }
}

impl<Fut> TaskGlobalExt for Fut where Fut: Future {}

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

#[pin_project]
pub struct TaskGlobalFuture<Fut, T> {
    #[pin]
    inner: Fut,
    value: Option<Box<TaskGlobalNode<T>>>,
}

impl<Fut, T> TaskGlobalFuture<Fut, T> {
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
