pub trait TaskGlobal {}

pub struct TaskGlobalIter /*<T>*/ {}

pub struct TaskGlobalIterMut /*<T>*/ {}

pub struct TaskGlobalFuture /*<F, T>*/ {}

#[allow(unused)]
pub struct TaskGlobalStorage<T> {
    head: Option<Box<TaskGlobalNode<T>>>,
}

#[allow(unused)]
impl<T> TaskGlobalStorage<T> {
    fn push(&mut self, mut node: Box<TaskGlobalNode<T>>) {
        node.parent = self.head.take();
        self.head = Some(node);
    }

    fn pop(&mut self) -> Option<Box<TaskGlobalNode<T>>> {
        let mut node = self.head.take();
        self.head = node.as_mut().and_then(|node| node.parent.take());
        node
    }
}

impl<T> Default for TaskGlobalStorage<T> {
    fn default() -> Self {
        Self { head: None }
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
struct TaskGlobalNodeGuard /*<T>*/ {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_can_push_and_pop() {
        let mut storage = TaskGlobalStorage::default();

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
