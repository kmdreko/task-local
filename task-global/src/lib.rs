use std::cell::{Ref, RefCell, RefMut};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::LocalKey;

use pin_project::pin_project;

pub trait TaskGlobal: Sized + 'static {
    fn key() -> &'static LocalKey<TaskGlobalStorage<Self>>;

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
}

pub struct TaskGlobalIter /*<T>*/ {}

pub struct TaskGlobalIterMut /*<T>*/ {}

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
        let mut this = self.project();
        let _guard = TaskGlobalNodeGuard::new(T::key(), &mut this.value);
        this.inner.poll(cx)
    }
}

pub struct TaskGlobalStorage<T> {
    head: RefCell<Option<Box<TaskGlobalNode<T>>>>,
}

impl<T> TaskGlobalStorage<T> {
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

impl<T> Default for TaskGlobalStorage<T> {
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
}

struct TaskGlobalNodeGuard<'a, T: 'static> {
    key: &'static LocalKey<TaskGlobalStorage<T>>,
    current: &'a mut Option<Box<TaskGlobalNode<T>>>,
}

impl<'a, T: 'static> TaskGlobalNodeGuard<'a, T> {
    fn new(
        key: &'static LocalKey<TaskGlobalStorage<T>>,
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
        let storage = TaskGlobalStorage::default();

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
        thread_local!(static STORAGE: TaskGlobalStorage<Context> = TaskGlobalStorage::default());
        struct Context;
        impl TaskGlobal for Context {
            fn key() -> &'static LocalKey<TaskGlobalStorage<Self>> {
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
            fn key() -> &'static LocalKey<TaskGlobalStorage<Self>> {
                thread_local!(static STORAGE: TaskGlobalStorage<Context> = TaskGlobalStorage::default());
                &STORAGE
            }
        }

        assert!(Context::try_global(|context| context.is_none()));
        assert!(Context::try_global_mut(|context| context.is_none()));
        TaskGlobalFuture::new(
            async {
                assert!(Context::try_global(|context| context.is_some()));
                assert!(Context::try_global_mut(|context| context.is_some()));
                assert!(Context::global(|context| matches!(context, Context)));
                assert!(Context::global_mut(|context| matches!(context, Context)));
            },
            Context,
        )
        .await;
        assert!(Context::try_global(|context| context.is_none()));
        assert!(Context::try_global_mut(|context| context.is_none()));
    }
}
