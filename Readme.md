Have you ever thought *"Dang! I sure would love to use global or thread-local
variables in my asynchronous tasks, but with all the thread-hopping and
cooperative-multi-tasking that'd be impossible"*? Well lament no more!
Because now you can...

# Task Local Variables

The `TaskLocal` trait allows you to add "local" variables to your tasks
and access them from anywhere within the task:

```
use task_local::{TaskLocal, WithLocalExt};

#[derive(Default, TaskLocal)]
struct Context {
    value: i32,
}

async {
    // set the local
    Context::local_mut(|ctx| ctx.value = 42);

    // get the local
    let value = Context::local(|ctx| ctx.value);
    println!("{}", value); // prints 42

}.with_local(Context::default()).await;
```

## Scoping

The local value is only available while the annotated task is executing.
It does not persist after the task has completed. It is not accessible while
the task is idle. It is not available from tasks "spawned" from the
annotated task. Annotated tasks can be nested and the value will be the one
set from the most-recently set scope.

```
assert!(Context::try_local(|_ctx| {}).is_err());

async {
    Context::local(|ctx| assert!(ctx.value == 0));
    Context::local_mut(|ctx| ctx.value = 42);
    Context::local(|ctx| assert!(ctx.value == 42));

    async {
        Context::local(|ctx| assert!(ctx.value == 0));
        Context::local_mut(|ctx| ctx.value = 5);
        Context::local(|ctx| assert!(ctx.value == 5));

    }.with_local(Context::default()).await;

    Context::local(|ctx| assert!(ctx.value == 42));

}.with_local(Context::default()).await;

assert!(Context::try_local(|_ctx| {}).is_err());
```

## How does it work?

While `Future`s are often moved between threads and their progress may be
interleaved with others, they cannot do so while they are executing. While
`.poll()` is running, nothing else can happen on that thread and the
`Future` cannot be moved or destroyed. This takes advantage of that by
storing a handle to the task-local value in thread-local storage that is
accessible by the free methods. It puts the handle in when `.poll()` starts,
and it takes it back out before it ends.

That could be the end of it, except nested uses of the annotated task would
stomp on the thread-local storage of their parent annotated tasks. This
avoids that problem by essentially storing a linked-list of handles, that
can be pushed and popped when tasks are suspended and resumed. Doing this
also allows access to prior scopes via the `local_chain` methods, which
could be useful in certain situations.

## Should I actually use this?

You should follow the same general advice for global and thread-local
variables and avoid them if there is an alternative. However, sometimes
using a local variable is ergonomically advantageous, and this gives you
that option in an asynchronous context.

However, it was only after creating this library that I found that it was
already implemented as [`tokio::task::LocalKey`](https://docs.rs/tokio/latest/tokio/task/struct.LocalKey.html).
The APIs are different, but largely implement the same functionality. The tokio
implementation uses a macro similar to `thread_local!` and has synchronous
support. The features my implementation has over it is mutability and the 
ability to access the whole stack of task-local values.

So probably not, but maybe. You can if you want. :)

