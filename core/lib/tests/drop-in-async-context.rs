#[macro_use] extern crate rocket;

use rocket::{Rocket, Build, build, fairing::AdHoc, Orbit};

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
    build().manage(AsyncDropInAsync).attach(AdHoc::on_liftoff(
        "Shutdown immediately",
        |rocket: &Rocket<Orbit>| Box::pin(async {
            rocket.shutdown().notify();
        }
    )))
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
    #[rocket::main]
    async fn main() {
        super::rocket().launch().await.unwrap();
    }
    #[test]
    fn test_main() {
        main();
    }
    #[test]
    fn test_execute() {
        rocket::execute(super::rocket().launch()).unwrap();
    }
}
