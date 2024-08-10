#[macro_use]
extern crate rocket;

use figment::Figment;
use rocket::{custom, fairing::AdHoc, Build, Orbit, Rocket, config::SecretKey};

struct AsyncDropInAsync;

impl Drop for AsyncDropInAsync {
    fn drop(&mut self) {
        // Attempt to fetch the current runtime while dropping
        // Pools in rocket_sync_db_pools (and maybe rocket_db_pools)
        // do use this capability. They spawn tasks to asyncronously
        // complete shutdown of the pool, which triggers the same panic.
        let _ = rocket::tokio::runtime::Handle::current();
    }
}

fn rocket() -> Rocket<Build> {
    let mut config = rocket::Config::default();
    config.secret_key = SecretKey::generate().unwrap();
    let figment = Figment::from(config);
    custom(figment).manage(AsyncDropInAsync).attach(AdHoc::on_liftoff(
        "Shutdown immediately",
        |rocket: &Rocket<Orbit>| {
            Box::pin(async {
                rocket.shutdown().notify();
            })
        },
    ))
}

mod launch {
    #[launch]
    fn launch() -> _ {
        super::rocket()
    }
    #[test]
    fn test_launch() {
        main();
    }
}

mod main {
    use rocket::tokio::net::TcpListener;

    #[rocket::main]
    async fn main() {
        super::rocket()
            .try_launch_on(TcpListener::bind("localhost:8001"))
            .await
            .unwrap();
    }
    #[test]
    fn test_main() {
        main();
    }
    #[test]
    fn test_execute() {
        rocket::execute(async {
            super::rocket()
                .try_launch_on(TcpListener::bind("localhost:8002"))
                .await
                .unwrap();
        });
    }
}
