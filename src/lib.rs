use std::{
    any::Any,
    future::Future,
    marker::PhantomData,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

pub trait FutureInheritTaskLocal: Future {
    fn inherit_task_local(self) -> impl Future<Output = Self::Output> + Send + Sync + 'static;
}

impl<F> FutureInheritTaskLocal for F
where
    F: Future + Send + Sync + 'static,
{
    fn inherit_task_local(self) -> impl Future<Output = Self::Output> + Send + Sync + 'static {
        let mut this = Some(self); // Only one of the two paths will execute, but the borrow checker doesn't know that.
        INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let new_task_locals = task_locals.clone();
                INHERITABLE_TASK_LOCALS.scope(new_task_locals, this.take().unwrap())
            })
            .unwrap_or_else(|_| {
                INHERITABLE_TASK_LOCALS.scope(
                    vec![None; NEXT_KEY.load(Ordering::Relaxed)],
                    this.take().unwrap(),
                )
            })
    }
}

tokio::task_local! {
    static INHERITABLE_TASK_LOCALS: Vec<Option<Arc<dyn Any + Send + Sync + 'static>>>
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

    pub fn scope<F>(&'static self, value: T, f: F) -> impl Future<Output = F::Output>
    where
        F: Future,
    {
        let mut value = Some((value, f));
        INHERITABLE_TASK_LOCALS
            .try_with(|task_locals| {
                let (value, f) = value.take().unwrap();
                let mut new_task_locals = task_locals.clone();
                new_task_locals[self.key] = Some(Arc::new(value) as Arc<_>);
                INHERITABLE_TASK_LOCALS.scope(new_task_locals, f)
            })
            .unwrap_or_else(|_| {
                let (value, f) = value.take().unwrap();
                let mut new_task_locals = vec![None; NEXT_KEY.load(Ordering::Relaxed)];
                new_task_locals[self.key] = Some(Arc::new(value) as Arc<_>);
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
                new_task_locals[self.key] = Some(Arc::new(value) as Arc<_>);
                new_task_locals
            })
            .unwrap_or_else(|_| {
                let value = value.take().unwrap();
                let mut new_task_locals = vec![None; NEXT_KEY.load(Ordering::Relaxed)];
                new_task_locals[self.key] = Some(Arc::new(value) as Arc<_>);
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
                .get(self.key)
                .expect(
                    "task local vec was the wrong length, this is a tokio-inherit-task-local bug",
                )
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
                .get(self.key)
                .expect(
                    "task local vec was the wrong length, this is a tokio-inherit-task-local bug",
                )
                .as_ref()
                .ok_or(InheritableAccessError::NotInHashmap)?;
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum InheritableAccessError {
    NotInHashmap,
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
