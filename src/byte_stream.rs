use std::boxed::Box;
use std::cell::UnsafeCell;
use std::error::Error;
use std::io;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_web::web::Bytes;
use futures::stream::BoxStream;
use futures::Stream;
use serde::Serialize;
use sqlx::{Database, FromRow, Sqlite, SqlitePool};

pub struct ByteStream<'s, R> {
    pool: UnsafeCell<SqlitePool>,
    sql: UnsafeCell<Pin<String>>,
    results: u32,
    stream: Option<BoxStream<'s, Result<R, sqlx::Error>>>,
    buf: Vec<u8>,
    _pinned: PhantomPinned,
}

impl<'s, R> ByteStream<'s, R>
where
    R: for<'r> FromRow<'r, <Sqlite as Database>::Row> + Send + Unpin + 's,
{
    pub fn new(
        pool: &'_ SqlitePool,
        sql: &str,
        results: Option<u32>,
    ) -> Pin<Box<ByteStream<'s, R>>> {
        println!("ByteStream::new()");
        let wrapped = ByteStream {
            pool: UnsafeCell::new(pool.clone()),
            results: results.or(Some(10)).unwrap(),
            sql: UnsafeCell::new(Pin::new(String::from(sql))),
            stream: None,
            buf: Vec::new(),
            _pinned: PhantomPinned,
        };
        let mut wrapped = Box::pin(wrapped);

        // Safety: Shared access to this.pool is safe because we are using a
        // cloned copy of the argument pool, and SqlitePool is a newtype around
        // and Arc containing the connection. It will continue to exist so long
        // as our ownership in boxed.pool exists.
        let pool = unsafe { &*wrapped.pool.get() };
        // Safety: same as pool
        let sql = unsafe { &**wrapped.sql.get() };
        let stream: BoxStream<'s, Result<R, sqlx::Error>> = Box::pin(
            sqlx::query_as::<Sqlite, R>(sql)
                .bind(wrapped.results)
                .fetch(pool),
        );

        // Safety: We haven't returned our new instance of HoldUp yet,
        // therefore we have no references. Exclusive access is safe.
        unsafe {
            let mut_ref = Pin::get_unchecked_mut(wrapped.as_mut());
            mut_ref.stream = Some(stream);
        }

        wrapped
    }
}

impl<'s, R> Stream for ByteStream<'s, R>
where
    R: Serialize,
{
    type Item = Result<Bytes, Box<dyn Error>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safety: XXX What makes this use safe? XXX
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(stream) = &mut this.stream {
            match stream.as_mut().poll_next(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Some(Ok(model))) => {
                    let res = serde_json::to_writer(&mut this.buf, &model);
                    if let Err(e) = res {
                        return Poll::Ready(Some(Err(Box::new(e))));
                    }

                    let poll = Poll::Ready(Some(Ok(Bytes::copy_from_slice(&this.buf))));
                    this.buf.clear();
                    poll
                }
                Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(Box::new(e)))),
                Poll::Ready(None) => Poll::Ready(None),
            }
        } else {
            Poll::Ready(Some(Err(Box::new(io::Error::new(
                io::ErrorKind::Other,
                "ByteStream: expected stream to be Some",
            )))))
        }
    }
}
