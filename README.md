# tokio-inherit-task-local

Provides functionality very similar to [`tokio::task_local`](https://docs.rs/tokio/latest/tokio/macro.task_local.html) with one key difference. Any future annotated with
`.inherit_task_local()` will inherit the task local values of the task which spawned it. This does not inherit
values created by [`tokio::task_local`](https://docs.rs/tokio/latest/tokio/macro.task_local.html), it will only inherit values created by `inheritable_task_local`.

Here's a simple example

```rust
use tokio_inherit_task_local::{inheritable_task_local, FutureInheritTaskLocal as _};

inheritable_task_local! {
    pub static DEMO_VALUE: u32;
}

async fn foo() {
    let out = DEMO_VALUE
    .scope(5, async {
        tokio::spawn(async { DEMO_VALUE.with(|&v| v) }.inherit_task_local()).await
    })
    .await
    .unwrap();
    assert_eq!(out, 5);
}
```

Even though `DEMO_VALUE` was not defined for the spawned future, it was still able to inherit the value defined in
its parent. This happens thanks to the `.inherit_task_local()` method call. That method can be found in
`FutureInheritTaskLocal`.

These inherited values ***DO NOT*** need to be `Clone`. Child tasks will inherit counted references to the original value, so the value provided is never
cloned.

This crate does not support being used from inside of a DLL, .so file, .dylib, or any other kind
of runtime linked configuration. This crate assumes all inheritable task local declarations were available at
compile time. Dynamically linked projects may work by accident, but their behavior is not guaranteed.

Additionally this crate depends on [`ctor`](https://crates.io/crates/ctor) and therefore it is subject to the same platform limitations as [`ctor`](https://crates.io/crates/ctor).
