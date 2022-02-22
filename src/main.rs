use std::io;

use actix_web::{get, web, App, HttpResponse, HttpServer, Responder, Result as ActixResult};
use futures::TryStreamExt;
use serde::Deserialize;
use sqlx::SqlitePool;

mod byte_stream;
mod user;
mod wrapped_future;
mod wrapped_stream;

#[derive(Deserialize)]
struct ListUsersParams {
    results: Option<u32>,
}

#[get("/")]
async fn list_users(
    pool: web::Data<SqlitePool>,
    params: web::Query<ListUsersParams>,
) -> ActixResult<impl Responder> {
    let stream = byte_stream::ByteStream::<user::User>::new(
        &pool,
        "WITH RECURSIVE ids(id) AS ( \
            SELECT ABS(RANDOM() % 100) + 1 \
            UNION ALL \
            SELECT ABS(RANDOM() % 100) + 1 FROM ids LIMIT ? \
         )
         SELECT users.id, users.name, users.email \
         FROM users \
         JOIN ids ON users.id = ids.id",
        params.results,
    );
    Ok(HttpResponse::Ok().streaming(stream))
}

async fn actix_example(pool: SqlitePool) -> Result<(), io::Error> {
    println!("\nStarting a webserver on http://127.0.0.1:3000/");
    println!("Press control-c to quit");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(list_users)
    })
    .bind("127.0.0.1:3000")?
    .run()
    .await
}

#[tokio::main]
async fn main() -> Result<(), sqlx::Error> {
    env_logger::init();
    let pool = SqlitePool::connect("sqlite://dev.db")
        .await
        .expect("connected to dev.db");

    println!("Example using a future owning all data wrapping another future using that data");
    let query = wrapped_future::WrappedFuture::new(&pool, "Dane Petty");
    let r = query.await;
    match r {
        Ok(row) => println!(
            "row: {}, {}, {}",
            sqlx::Row::get::<u32, _>(&row, 0),
            sqlx::Row::get::<&str, _>(&row, 1),
            sqlx::Row::get::<&str, _>(&row, 2)
        ),
        Err(e) => println!("error: {:?}", &e),
    }

    println!("\nExample using a stream owning all data wrapping another stream using that data");
    let mut stream = wrapped_stream::WrappedStream::new(&pool, "%v%");
    while let Some(row) = stream.try_next().await? {
        println!(
            "row: {}, {}, {}",
            sqlx::Row::get::<u32, _>(&row, 0),
            sqlx::Row::get::<&str, _>(&row, 1),
            sqlx::Row::get::<&str, _>(&row, 2)
        );
    }

    actix_example(pool.clone()).await?;

    Ok(())
}
