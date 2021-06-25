use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::Poll;
pub trait NonblockingFuture {
    type Output;
    fn poll_nb(self: Pin<&mut Self>) -> Poll<Self::Output>;
}

pub trait NonblockingFutureExt: NonblockingFuture {
    fn poll_nb_unpin(&mut self) -> Poll<Self::Output>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_nb()
    }
}
impl<T: NonblockingFuture> NonblockingFutureExt for T {}

impl<F: ?Sized + NonblockingFuture + Unpin> NonblockingFuture for &mut F {
    type Output = F::Output;

    fn poll_nb(mut self: Pin<&mut Self>) -> Poll<Self::Output> {
        F::poll_nb(Pin::new(&mut **self))
    }
}

impl<P> NonblockingFuture for Pin<P>
where
    P: Unpin + DerefMut,
    P::Target: NonblockingFuture,
{
    type Output = <<P as Deref>::Target as NonblockingFuture>::Output;

    fn poll_nb(self: Pin<&mut Self>) -> Poll<Self::Output> {
        Pin::get_mut(self).as_mut().poll_nb()
    }
}

#[macro_export]
macro_rules! nb_impl_future {
    ($t: ident) => {
        impl std::future::Future for $t {
            type Output = <$t as NonblockingFuture>::Output;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                cx.waker().wake_by_ref();
                self.poll_nb()
            }
        }
    };
}
