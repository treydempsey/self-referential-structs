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
    state: StreamState,
    _pinned: PhantomPinned,
}

enum StreamState {
    Unstarted,
    FirstRecord,
    AdditionalRecords,
    Closed,
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
        let wrapped = ByteStream {
            pool: UnsafeCell::new(pool.clone()),
            results: results.or(Some(10)).unwrap(),
            sql: UnsafeCell::new(Pin::new(String::from(sql))),
            stream: None,
            buf: Vec::new(),
            state: StreamState::Unstarted,
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

        // Safety: We haven't returned our new instance of Self yet,
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
        // Safety: This is safe because we are Pin and our fields are Pin. None of our
        // fields are public so we can guarantee our fields won't move.
        let this = unsafe { self.get_unchecked_mut() };

        if matches!(this.state, StreamState::Closed) {
            return Poll::Ready(None);
        } else if matches!(this.state, StreamState::Unstarted) {
            this.buf.append(&mut vec![b'[']);
            this.state = StreamState::FirstRecord;
        }

        if let Some(stream) = &mut this.stream {
            let mut count = 0;

            // Fill this.buf with up to 100 rows
            let mut poll = stream.as_mut().poll_next(cx);
            while let Poll::Ready(Some(Ok(ref model))) = poll {
                // For every additional record after the first precede it with a comma
                if matches!(this.state, StreamState::FirstRecord) {
                    this.state = StreamState::AdditionalRecords;
                } else {
                    this.buf.append(&mut vec![b',']);
                }

                let res = serde_json::to_writer(&mut this.buf, &model);
                if let Err(e) = res {
                    return Poll::Ready(Some(Err(Box::new(e))));
                }

                count += 1;
                if count == 100 {
                    break;
                }

                poll = stream.as_mut().poll_next(cx);
            }

            match poll {
                Poll::Ready(Some(Ok(_))) | Poll::Ready(None) | Poll::Pending if count != 0 => {
                    if matches!(poll, Poll::Ready(None)) {
                        this.state = StreamState::Closed;
                        this.buf.append(&mut vec![b']']);
                    }

                    let poll = Poll::Ready(Some(Ok(Bytes::copy_from_slice(&this.buf))));
                    this.buf.clear();
                    poll
                }
                Poll::Pending => Poll::Pending,
                Poll::Ready(None) => {
                    this.state = StreamState::Closed;
                    this.buf.append(&mut vec![b']']);
                    let poll = Poll::Ready(Some(Ok(Bytes::copy_from_slice(&this.buf))));
                    this.buf.clear();
                    poll
                }
                Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(Box::new(e)))),
                Poll::Ready(Some(Ok(_))) => unreachable!(),
            }
        } else {
            Poll::Ready(Some(Err(Box::new(io::Error::new(
                io::ErrorKind::Other,
                "ByteStream: expected stream to be Some",
            )))))
        }
    }
}
