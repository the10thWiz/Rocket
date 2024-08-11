// use figment::Figment;
// use rocket::{custom, fairing::AdHoc, Build, Orbit, Rocket, config::SecretKey};

struct AssertDropInAsync;

impl Drop for AssertDropInAsync {
    fn drop(&mut self) {
        // Attempt to fetch the current runtime while dropping
        // Pools in rocket_sync_db_pools (and maybe rocket_db_pools)
        // do use this capability. They spawn tasks to asyncronously
        // complete shutdown of the pool, which triggers the same panic.
        let _ = rocket::tokio::runtime::Handle::current();
    }
}

// fn rocket() -> Rocket<Build> {
//     let mut config = rocket::Config::default();
//     config.secret_key = SecretKey::generate().unwrap();
//     // config.= 0;
//     let figment = Figment::from(config);
//     custom(figment).manage(AsyncDropInAsync).attach(AdHoc::on_liftoff(
//         "Shutdown immediately",
//         |rocket: &Rocket<Orbit>| {
//             Box::pin(async {
//                 rocket.shutdown().notify();
//             })
//         },
//     ))
// }

mod launch {
    use crate::prelude::*;
    use super::AssertDropInAsync;
    #[launch]
    fn launch() -> _ {
        rocket::build().manage(AssertDropInAsync)
    }
    fn run(token: (Token, ())) -> Launched {
        main();
        token.0.ignored()
    }
    fn test_launch() -> Result<()> {
        let mut server = Server::spawn((), run)?;
        server.terminate()?;
        Ok(())
    }
    register!(test_launch);
}

// mod main {
//     use rocket::tokio::net::TcpListener;

//     #[rocket::main]
//     async fn main() {
//         super::rocket()
//             .launch()
//             .await
//             .unwrap();
//     }
//     #[test]
//     fn test_main() {
//         main();
//     }
//     #[test]
//     fn test_execute() {
//         rocket::execute(async {
//             super::rocket()
//                 .launch()
//                 .await
//                 .unwrap();
//         });
//     }
// }
