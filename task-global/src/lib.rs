use std::cell::RefCell;
use std::thread::LocalKey;

pub trait TaskGlobal: Sized {
    fn key() -> &'static LocalKey<TaskGlobalStorage<Self>>;
}

pub struct TaskGlobalIter /*<T>*/ {}

pub struct TaskGlobalIterMut /*<T>*/ {}

pub struct TaskGlobalFuture /*<F, T>*/ {}

#[allow(unused)]
pub struct TaskGlobalStorage<T> {
    head: RefCell<Option<Box<TaskGlobalNode<T>>>>,
}

#[allow(unused)]
impl<T> TaskGlobalStorage<T> {
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

#[allow(unused)]
struct TaskGlobalNode<T> {
    value: T,
    parent: Option<Box<TaskGlobalNode<T>>>,
}

#[allow(unused)]
impl<T> TaskGlobalNode<T> {
    fn new(value: T) -> TaskGlobalNode<T> {
        TaskGlobalNode {
            value,
            parent: None,
        }
    }
}

#[allow(unused)]
struct TaskGlobalNodeGuard<'a, T: 'static> {
    key: &'static LocalKey<TaskGlobalStorage<T>>,
    current: &'a mut Option<Box<TaskGlobalNode<T>>>,
}

#[allow(unused)]
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
}
