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
