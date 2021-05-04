//! HTTP upgrade traits and objects.
//!
//! Executing an HTTP upgrade requires creating a response with the upgrade agent.

use async_trait::async_trait;
use rocket_http::hyper::upgrade::OnUpgrade;
use tokio::sync::oneshot;

/// Upgraded connection, implements AsyncRead + AsyncWrite for reading and writing
pub use rocket_http::hyper::upgrade::Upgraded;

/// Respresents an object that handles a connection after an HTTP upgrade has taken place.
///
/// TODO: add example
#[async_trait]
pub trait UpgradeResponder: Send + Unpin {
    async fn on_upgrade(self: Box<Self>, upgrade_obj: Upgraded) -> std::io::Result<()>;
}

/// Creates a pending upgrade task, if upgradable is Some.
///
/// If the return value is Some(tx), this function will wait for the `OnUpgrade` object, so sending
/// should never fail.
///
/// When upgradable is None, this does nothing.
pub(crate) fn upgrade_pending(upgradable: Option<Box<dyn UpgradeResponder>>)
        -> Option<oneshot::Sender<OnUpgrade>> {
    if let Some(upgradable) = upgradable {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            if let Ok(on_upgrade) = rx.await {
                match on_upgrade.await {
                    Ok(upgraded) => {
                        if let Err(e) = upgradable.on_upgrade(upgraded).await {
                            eprintln!("server foobar io error: {}", e)
                        }else {
                            println!("Socket handler finished");
                        };
                    }
                    Err(e) => eprintln!("upgrade error: {:?}", e),
                }
            }
        });
        Some(tx)
    }else {
        None
    }
}


