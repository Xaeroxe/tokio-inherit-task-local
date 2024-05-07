//! This crate does not support being used from inside of a DLL, .so file, .dylib, or any other kind
//! of runtime linked configuration. This crate assumes all inheritable task local declarations were available at
//! compile time.

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

#[derive(Clone)]
pub struct TaskLocalInheritableTable {
    inner: Vec<Option<Arc<(dyn Any + Send + Sync + 'static)>>>,
}

impl TaskLocalInheritableTable {
    fn new(inner: Vec<Option<Arc<(dyn Any + Send + Sync + 'static)>>>) -> Self {
        Self { inner }
    }
}

impl Debug for TaskLocalInheritableTable {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        // Omit the inner value on purpose. The debug print of it isn't very useful anyways.
        f.debug_struct("TaskLocalInheritableTable").finish()
    }
}

pub trait FutureInheritTaskLocal: Future + Sized {
    fn inherit_task_local(self) -> TaskLocalFuture<TaskLocalInheritableTable, Self>;
}

impl<F> FutureInheritTaskLocal for F
where
    F: Future + 'static,
{
    fn inherit_task_local(self) -> TaskLocalFuture<TaskLocalInheritableTable, Self> {
        let mut this = Some(self); // Only one of the two paths will execute, but the borrow checker doesn't know that.
        INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let new_task_locals = task_locals.clone();
                INHERITABLE_TASK_LOCALS.scope(new_task_locals, this.take().unwrap())
            })
            .unwrap_or_else(|_| {
                INHERITABLE_TASK_LOCALS.scope(
                    TaskLocalInheritableTable::new(Vec::new()),
                    this.take().unwrap(),
                )
            })
    }
}

tokio::task_local! {
    static INHERITABLE_TASK_LOCALS: TaskLocalInheritableTable
}

pub struct InheritableLocalKey<T: 'static> {
    key: usize,
    _phantom: PhantomData<T>,
}

impl<T: Send + Sync> InheritableLocalKey<T> {
    #[doc(hidden)]
    pub fn _new() -> Self {
        Self {
            key: NEXT_KEY.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed),
            _phantom: ::std::marker::PhantomData,
        }
    }

    pub fn scope<F>(&'static self, value: T, f: F) -> TaskLocalFuture<TaskLocalInheritableTable, F>
    where
        F: Future,
    {
        let mut value = Some((value, f));
        INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let (value, f) = value.take().unwrap();
                let mut new_task_locals = task_locals.clone();
                maybe_init_task_locals(&mut new_task_locals);
                new_task_locals.inner[self.key] = Some(Arc::new(value) as Arc<_>);
                INHERITABLE_TASK_LOCALS.scope(new_task_locals, f)
            })
            .unwrap_or_else(|_| {
                let (value, f) = value.take().unwrap();
                let mut new_task_locals = new_task_local_table();
                new_task_locals.inner[self.key] = Some(Arc::new(value) as Arc<_>);
                INHERITABLE_TASK_LOCALS.scope(new_task_locals, f)
            })
    }

    pub fn sync_scope<F, R>(&'static self, value: T, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let mut value = Some(value);
        let new_task_locals = INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let value = value.take().unwrap();
                let mut new_task_locals = task_locals.clone();
                maybe_init_task_locals(&mut new_task_locals);
                new_task_locals.inner[self.key] = Some(Arc::new(value) as Arc<_>);
                new_task_locals
            })
            .unwrap_or_else(|_| {
                let value = value.take().unwrap();
                let mut new_task_locals = new_task_local_table();
                new_task_locals.inner[self.key] = Some(Arc::new(value) as Arc<_>);
                new_task_locals
            });
        INHERITABLE_TASK_LOCALS.sync_scope(new_task_locals, f)
    }

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

    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, InheritableAccessError>
    where
        F: FnOnce(&T) -> R,
    {
        let r = INHERITABLE_TASK_LOCALS.try_with(|task_locals| {
            let v = task_locals
                .inner
                .get(self.key)
                .ok_or(InheritableAccessError::NotInVec)?
                .as_ref()
                .ok_or(InheritableAccessError::NotInVec)?;
            Result::<_, InheritableAccessError>::Ok((f)(v.downcast_ref::<T>().expect(
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

fn new_task_local_table() -> TaskLocalInheritableTable {
    TaskLocalInheritableTable::new(vec![None; NEXT_KEY.load(Ordering::Relaxed)])
}

fn maybe_init_task_locals(new_task_locals: &mut TaskLocalInheritableTable) {
    if new_task_locals.inner.is_empty() {
        *new_task_locals = new_task_local_table();
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum InheritableAccessError {
    NotInVec,
    NotInTokio,
}

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
