#[cfg(feature = "glommio")]
mod glommio_tp;
#[cfg(feature = "glommio")]
mod glommio_static;
#[cfg(feature = "tokio")]
mod tokio_static;

#[cfg(feature = "glommio")]
pub use glommio_tp::*;
#[cfg(feature = "glommio")]
pub use glommio_static::*;
#[cfg(feature = "tokio")]
pub use tokio_static::*;
