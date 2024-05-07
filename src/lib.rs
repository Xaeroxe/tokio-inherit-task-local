//! Provides functionality very similar to [`tokio::task_local`] with one key difference. Any future annotated with
//! [`.inherit_task_local()`](FutureInheritTaskLocal::inherit_task_local) will inherit the task local values of the task which spawned it. This does not inherit
//! values created by [`tokio::task_local`], it will only inherit values created by [`inheritable_task_local`].
//!
//! Here's a simple example
//!
//! ```
//! # use tokio_inherit_task_local::inheritable_task_local;
//!
//! use tokio_inherit_task_local::FutureInheritTaskLocal as _;
//!
//! inheritable_task_local! {
//!     pub static DEMO_VALUE: u32;
//! }
//!
//! async fn foo() {
//!     let out = DEMO_VALUE
//!         .scope(5, async {
//!            tokio::spawn(async { DEMO_VALUE.with(|&v| v) }.inherit_task_local()).await
//!         })
//!         .await
//!         .unwrap();
//!     assert_eq!(out, 5);
//! }
//! ```
//!
//! Even though `DEMO_VALUE` was not defined for the spawned future, it was still able to inherit the value defined in
//! its parent. This happens thanks to the [`.inherit_task_local()`](FutureInheritTaskLocal::inherit_task_local) method call. That method can be found in
//! [`FutureInheritTaskLocal`].
//!
//! These inherited values ***DO NOT*** need to be [`Clone`]. Child tasks will inherit counted references to the original value, so the value provided is never
//! cloned.
//!
//! This crate does not support being used from inside of a DLL, .so file, .dylib, or any other kind
//! of runtime linked configuration. This crate assumes all inheritable task local declarations were available at
//! compile time. Dynamically linked projects may work by accident, but their behavior is not guaranteed.
//!
//! Additionally this crate depends on [`ctor`] and therefore it is subject to the same platform limitations as [`ctor`].

use std::{
    any::Any,
    fmt::{Debug, Formatter, Result as FmtResult},
    future::Future,
    marker::PhantomData,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use tokio::task::futures::TaskLocalFuture;

/// This is mostly an implementation detail. It stores references to all of the inheritable task local values that are available to
/// a given task. You are not meant to use this directly.
#[derive(Clone)]
pub struct TaskLocalInheritableTable {
    inner: Box<[Option<Arc<(dyn Any + Send + Sync + 'static)>>]>,
}

impl TaskLocalInheritableTable {
    fn new(inner: Box<[Option<Arc<(dyn Any + Send + Sync + 'static)>>]>) -> Self {
        Self { inner }
    }
}

impl Debug for TaskLocalInheritableTable {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        // Omit the inner value on purpose. The debug print of it isn't very useful anyways.
        f.debug_struct("TaskLocalInheritableTable").finish()
    }
}

/// Extends any [`Future`] with a `'static` lifetime. Provides a method that copies references to the current inheritable task local
/// values into this [`Future`].
pub trait FutureInheritTaskLocal: Future + Sized {
    /// Copies references to the inheritable task local values that are currently available into this [`Future`]. These
    /// copied references will be available after the [`Future`] has been spawned onto a [`tokio`] runtime.
    ///
    /// # Example
    ///
    /// ```
    /// # async fn func() {
    /// # let a_future = async { () };
    /// use tokio_inherit_task_local::FutureInheritTaskLocal as _;
    ///
    /// tokio::spawn(a_future.inherit_task_local());
    /// # }
    /// ```
    fn inherit_task_local(self) -> TaskLocalFuture<TaskLocalInheritableTable, Self>;
}

impl<F> FutureInheritTaskLocal for F
where
    F: Future + 'static,
{
    fn inherit_task_local(self) -> TaskLocalFuture<TaskLocalInheritableTable, Self> {
        let mut this = Some(self); // Only one of the two paths will execute, but the borrow checker doesn't know that.
        let new_task_locals = INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| task_locals.clone())
            .unwrap_or_else(|_| TaskLocalInheritableTable::new(Box::new([])));
        INHERITABLE_TASK_LOCALS.scope(new_task_locals, this.take().unwrap())
    }
}

tokio::task_local! {
    static INHERITABLE_TASK_LOCALS: TaskLocalInheritableTable
}

/// A key for inheritable task-local data.
///
/// This type is generated by the [`inheritable_task_local!`] macro.
///
/// Unlike [`std::thread::LocalKey`], `InheritableLocalKey` will
/// _not_ lazily initialize the value on first access. Instead, the
/// value is first initialized when the future containing
/// the task-local is first polled by a futures executor, like Tokio.
///
/// # Examples
///
/// ```
/// # async fn dox() {
/// # use tokio_inherit_task_local::inheritable_task_local;
/// inheritable_task_local! {
///     static NUMBER: u32;
/// }
///
/// NUMBER.scope(1, async move {
///     assert_eq!(NUMBER.get(), 1);
/// }).await;
///
/// NUMBER.scope(2, async move {
///     assert_eq!(NUMBER.get(), 2);
///
///     NUMBER.scope(3, async move {
///         assert_eq!(NUMBER.get(), 3);
///     }).await;
/// }).await;
/// # }
/// ```
///
/// [`std::thread::LocalKey`]: struct@std::thread::LocalKey
pub struct InheritableLocalKey<T: 'static> {
    key: usize,
    _phantom: PhantomData<T>,
}

impl<T: Send + Sync> InheritableLocalKey<T> {
    #[doc(hidden)]
    pub fn _new() -> Self {
        Self {
            key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
            _phantom: PhantomData,
        }
    }

    /// Sets a value `T` as the inheritable task-local value for the future `F`.
    ///
    /// Once this future and all of its inheriting descendants have completed, the value
    /// will be dropped.
    ///
    /// ### Panics
    ///
    /// If you poll any future returned by this method inside a call to [`with`] or
    /// [`try_with`] then the call to `poll` will panic.
    ///
    /// ### Examples
    ///
    /// ```
    /// # async fn dox() {
    /// # use tokio_inherit_task_local::inheritable_task_local;
    /// inheritable_task_local! {
    ///     static NUMBER: u32;
    /// }
    ///
    /// NUMBER.scope(1, async move {
    ///     println!("task local value: {}", NUMBER.get());
    /// }).await;
    /// # }
    /// ```
    ///
    /// [`with`]: fn@Self::with
    /// [`try_with`]: fn@Self::try_with
    pub fn scope<F>(&'static self, value: T, f: F) -> TaskLocalFuture<TaskLocalInheritableTable, F>
    where
        F: Future,
    {
        let mut new_task_locals = INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let mut new_task_locals = task_locals.clone();
                maybe_init_task_locals(&mut new_task_locals);
                new_task_locals
            })
            .unwrap_or_else(|_| new_task_local_table());
        new_task_locals.inner[self.key] = Some(Arc::new(value) as Arc<_>);
        INHERITABLE_TASK_LOCALS.scope(new_task_locals, f)
    }

    /// Sets a value `T` as the inheritable task-local value for the closure `F`.
    ///
    /// On completion of `sync_scope`, the task-local will be dropped, unless the closure
    /// spawned a task which inherited this value.
    ///
    /// ### Panics
    ///
    /// This method panics if called inside a call to [`with`] or [`try_with`]
    ///
    /// ### Examples
    ///
    /// ```
    /// # async fn dox() {
    /// # use tokio_inherit_task_local::inheritable_task_local;
    /// inheritable_task_local! {
    ///     static NUMBER: u32;
    /// }
    ///
    /// NUMBER.sync_scope(1, || {
    ///     println!("task local value: {}", NUMBER.get());
    /// });
    /// # }
    /// ```
    ///
    /// [`with`]: fn@Self::with
    /// [`try_with`]: fn@Self::try_with
    pub fn sync_scope<F, R>(&'static self, value: T, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let mut new_task_locals = INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let mut new_task_locals = task_locals.clone();
                maybe_init_task_locals(&mut new_task_locals);
                new_task_locals
            })
            .unwrap_or_else(|_| new_task_local_table());
        new_task_locals.inner[self.key] = Some(Arc::new(value) as Arc<_>);
        INHERITABLE_TASK_LOCALS.sync_scope(new_task_locals, f)
    }

    /// Accesses the current inheritable task-local and runs the provided closure.
    ///
    /// # Panics
    ///
    /// This function will panic if the task local doesn't have a value set.
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        INHERITABLE_TASK_LOCALS.with(|task_locals| {
            let v = task_locals
                .inner
                .get(self.key)
                .expect("no inheritable task locals are defined")
                .as_ref()
                .expect("inheritable task local was not defined");
            (f)(v
                .downcast_ref::<T>()
                .expect("internal was not of correct type, this is a tokio-inherit-task-local bug"))
        })
    }

    /// Accesses the current inheritable task-local and runs the provided closure.
    ///
    /// If the task-local with the associated key is not present, this
    /// method will return an `InheritableAccessError`. For a panicking variant,
    /// see `with`.
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, InheritableAccessError>
    where
        F: FnOnce(&T) -> R,
    {
        let r = INHERITABLE_TASK_LOCALS.try_with(|task_locals| {
            if task_locals.inner.is_empty() {
                return Err(InheritableAccessError::TableEmpty);
            }
            let v = task_locals
                .inner
                .get(self.key)
                .ok_or(InheritableAccessError::InvalidKey)?
                .as_ref()
                .ok_or(InheritableAccessError::NotInTable)?;
            Ok((f)(v.downcast_ref::<T>().expect(
                "internal was not of correct type, this is a tokio-inherit-task-local bug",
            )))
        });
        match r {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(InheritableAccessError::NotInTokio),
        }
    }
}

impl<T: Clone + Send + Sync> InheritableLocalKey<T> {
    /// Returns a copy of the inheritable task-local value
    /// if the task-local value implements `Clone`.
    ///
    /// # Panics
    ///
    /// This function will panic if the task local doesn't have a value set.
    pub fn get(&'static self) -> T {
        self.with(|v| v.clone())
    }
}

fn new_task_local_table() -> TaskLocalInheritableTable {
    TaskLocalInheritableTable::new(vec![None; NEXT_KEY.load(Ordering::Relaxed)].into_boxed_slice())
}

fn maybe_init_task_locals(new_task_locals: &mut TaskLocalInheritableTable) {
    if new_task_locals.inner.is_empty() {
        *new_task_locals = new_task_local_table();
    }
}

/// Returned when the requested inheritable task local did not have a value set.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum InheritableAccessError {
    /// Inheritable task locals are available to this future, however this key doesn't have a corresponding value.
    NotInTable,
    /// Inheritable task locals are initialized, however none of them have been set.
    TableEmpty,
    /// Inheritable task locals are initialized, however there is no slot that corresponds to this key.
    /// This error should not occur unless the API is used in an unsupported way.
    InvalidKey,
    /// Inheritable task locals are not initialized for this future at all.
    NotInTokio,
}

/// Declares a new inheritable task-local key of type [`InheritableLocalKey`].
///
/// # Syntax
///
/// The macro wraps any number of static declarations and makes them local to the current task.
/// Publicity and attributes for each static is preserved. For example:
///
/// # Examples
///
/// ```
/// # use tokio_inherit_task_local::inheritable_task_local;
/// inheritable_task_local! {
///     pub static ONE: u32;
///
///     #[allow(unused)]
///     static TWO: f32;
/// }
/// # fn main() {}
/// ```
///
/// See [`InheritableLocalKey` documentation][`InheritableLocalKey`] for more
/// information.
///
/// [`InheritableLocalKey`]: struct@InheritableLocalKey
#[macro_export]
macro_rules! inheritable_task_local {
    // empty (base case for the recursion)
   () => {};

   ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty; $($rest:tt)*) => {
       $crate::__inheritable_task_local_inner!($(#[$attr])* $vis $name, $t);
       $crate::inheritable_task_local!($($rest)*);
   };

   ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty) => {
       $crate::__inheritable_task_local_inner!($(#[$attr])* $vis $name, $t);
   }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __inheritable_task_local_inner {
   ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty) => {
       $(#[$attr])*
       #[$crate::ctor::ctor]
       $vis static $name: $crate::InheritableLocalKey<$t> = $crate::InheritableLocalKey::_new();
   };
}

static NEXT_KEY: AtomicUsize = AtomicUsize::new(0);

#[doc(hidden)]
pub use ctor;
