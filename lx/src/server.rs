use std::{
   future::Future,
   io,
   net::SocketAddr,
   path::{Path, PathBuf},
   pin::pin,
   time::Duration,
};

use axum::{
   extract::{
      ws::{Message, Utf8Bytes, WebSocket},
      State, WebSocketUpgrade,
   },
   response::Response,
   routing::{self},
   Router,
};
use futures::{
   future::{self, Either},
   SinkExt, StreamExt,
};
use log::{debug, error, info, trace};
use lx_md::Markdown;
use notify::RecursiveMode;
use notify_debouncer_full::DebouncedEvent;
use serde::Serialize;
use tokio::{
   net::TcpListener,
   runtime::Runtime,
   sync::{
      broadcast::{self, error::RecvError, Sender},
      mpsc,
   },
   task::JoinError,
};
use tower_http::services::{ServeDir, ServeFile};

// Initially, just rebuild everything. This can get smarter later!
use crate::build::{self, config_for};

/// Serve the site, blocking on the result (i.e. blocking forever until it is
/// killed by some kind of signal or failure).
pub fn serve(site_dir: &Path) -> Result<(), Error> {
   // Instead of making `main` be `async` (regardless of whether it needs it, as
   // many operations do *not*), make *this* function handle it. An alternative
   // would be to do this same basic wrapping in `main` but only for this.
   let rt = Runtime::new().map_err(|e| Error::Io { source: e })?;

   // This does not presently change for any reason. In principle it *could*, e.g. if I
   // wanted to reload it when config changed to support reloading syntaxes. For now,
   // though, this is sufficient.
   let md = Markdown::new(None);

   // 1. Run an initial build.
   // 2. Create a watcher on the *input* directory, *not* the output directory.
   // 3. When the watcher signals a change, use that to trigger a new *build*, not a
   //    reload.
   // 4. When the build finishes, use *that* to trigger a reload.
   let site_dir = site_dir.try_into()?;
   trace!("Building in {site_dir:?}");
   let config = config_for(&site_dir)?; // TODO: watch this separately?
   trace!("Computed config: {config:?}");
   build::build(site_dir, &config, &md).map_err(Error::from)?;

   // I only need the tx side, since I am going to take advantage of the fact that
   // `broadcast::Sender` implements `Clone` to pass it around and get easy and convenient
   // access to local receivers with `tx.subscribe()`.
   let (tx, _rx) = broadcast::channel(10);

   let serve_handle = rt.spawn(serve_in(config.output.clone(), tx.clone()));
   let watch_handle = rt.spawn(watch_in(config.output.clone(), tx.clone()));
   // TODO: actually *use* the handle!
   let _rebuild_handle = (); // TODO

   match rt.block_on(race_all([serve_handle, watch_handle])) {
      Ok(Ok(_)) => Ok(()),
      Ok(Err(reason)) => Err(reason),
      Err(join_error) => Err(Error::Serve { source: join_error }),
   }
}

async fn serve_in(path: PathBuf, state: Tx) -> Result<(), Error> {
   // This could be extracted into its own function.
   let serve_dir = ServeDir::new(&path).append_index_html_on_directories(true);
   let router = Router::new()
      .route_service("/", ServeFile::new(path.join("index.html")))
      .route_service("/*asset", serve_dir)
      .route("/live-reload", routing::get(websocket_upgrade))
      .with_state(state);

   let addr = SocketAddr::from(([127, 0, 0, 1], 24747)); // 24747 = CHRIS on a phone 🤣
   let listener = TcpListener::bind(addr)
      .await
      .map_err(|e| Error::BadAddress {
         value: addr,
         source: e,
      })?;

   info!("→ Serving\n\tat: http://{addr}\n\tfrom {}", path.display());

   axum::serve(listener, router)
      .await
      .map_err(|source| Error::ServeStart { source })
}

async fn websocket_upgrade(
   extractor: WebSocketUpgrade,
   State(state): State<Tx>,
) -> Response {
   debug!("binding websocket upgrade");
   extractor.on_upgrade(|socket| {
      debug!("upgrading the websocket");
      websocket(socket, state)
   })
}

async fn websocket(socket: WebSocket, state: Sender<Change>) {
   let (mut ws_tx, mut ws_rx) = socket.split();
   let mut change_rx = state.subscribe();

   let reload = pin!(async {
      loop {
         match change_rx.recv().await {
            Ok(Change { paths }) => {
               let paths_desc = paths
                  .iter()
                  .map(|p| p.to_string_lossy())
                  .collect::<Vec<_>>()
                  .join("\n\t");
               debug!("sending WebSocket reload message with paths:\n\t{paths_desc}");

               let payload = serde_json::to_string(&ChangePayload::Reload { paths })
                  .unwrap_or_else(|e| panic!("Could not serialize payload: {e}"));

               match ws_tx.send(Message::Text(payload.into())).await {
                  Ok(_) => debug!("Successfully sent {paths_desc}"),
                  Err(reason) => error!("Could not send WebSocket message:\n{reason}"),
               }
            }

            Err(recv_error) => match recv_error {
               RecvError::Closed => break,
               RecvError::Lagged(skipped) => {
                  error!("Websocket change notifier: lost {skipped} messages");
               }
            },
         }
      }
   });

   let close = pin!(async {
      while let Some(message) = ws_rx.next().await {
         match handle(message) {
            Ok(state) => debug!("{state}"),

            Err(error) => {
               debug!("WebSocket error:\n{error}");
               break;
            }
         }
      }
   });

   (reload, close).race().await;
}

fn handle(message_result: Result<Message, axum::Error>) -> Result<WebSocketState, Error> {
   debug!("Received {message_result:?} from WebSocket.");

   use Message::*;
   match message_result {
      Ok(message) => match message {
         Text(content) => Err(Error::WebSocket(WebSocketError::UnexpectedText(
            content.to_string(),
         ))),

         Binary(content) => Err(Error::WebSocket(WebSocketError::UnexpectedBytes(
            content.into(),
         ))),

         Ping(bytes) => {
            debug!("Ping with bytes: {bytes:?}");
            Ok(WebSocketState::Open)
         }

         Pong(bytes) => {
            debug!("Ping with bytes: {bytes:?}");
            Ok(WebSocketState::Open)
         }

         Close(maybe_frame) => {
            let message = WebSocketState::Closed {
               reason: maybe_frame.map(|frame| {
                  let desc = if !frame.reason.is_empty() {
                     format!("Reason: {};", frame.reason)
                  } else {
                     String::from("")
                  };

                  let code = format!("Code: {}", frame.code);
                  desc + &code
               }),
            };

            Ok(message)
         }
      },

      Err(source) => Err(Error::WebSocket(WebSocketError::Receive { source })),
   }
}

#[derive(Debug, Serialize)]
enum ChangePayload {
   Reload { paths: Vec<PathBuf> },
}

#[derive(Debug)]
enum WebSocketState {
   Open,
   Closed { reason: Option<String> },
}

impl std::fmt::Display for WebSocketState {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      use WebSocketState::*;
      match self {
         Open => write!(f, "WebSocket state: open"),
         Closed {
            reason: Some(reason),
         } => write!(f, "WebSocket state: closed. Cause:\n{reason}"),
         Closed { reason: None } => write!(f, "WebSocket state: closed."),
      }
   }
}

#[derive(Debug, Clone)]
struct Change {
   pub paths: Vec<PathBuf>,
}

/// Shorthand for typing!
type Tx = Sender<Change>;

async fn watch_in(dir: PathBuf, change_tx: Tx) -> Result<(), Error> {
   let (tx, mut rx) = mpsc::channel(256);

   // Doing this here means we will not drop the watcher until this function
   // ends, and the `while let` below will continue until there is an error (or
   // something else shuts down the whole system here!).
   let mut debouncer = notify_debouncer_full::new_debouncer(
      Duration::from_secs(1),
      /*tick_rate */ None,
      move |result| {
         if let Err(e) = tx.blocking_send(result) {
            eprintln!("Could not send event.\nError:{e}");
         }
      },
   )?;

   debouncer.watch(&dir, RecursiveMode::Recursive)?;

   while let Some(result) = rx.recv().await {
      let paths = result
         .map_err(Error::DebounceErrors)?
         .into_iter()
         .flat_map(|DebouncedEvent { event, .. }| event.paths)
         .collect::<Vec<_>>();

      let change = Change { paths };
      if let Err(e) = change_tx.send(change) {
         eprintln!("Error sending out: {e:?}");
      }
   }

   Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
   #[error("Build error: {source}")]
   Build {
      #[from]
      source: build::Error,
   },

   #[error(transparent)]
   Canonicalize(#[from] crate::canonicalized::InvalidDir),

   #[error("I/O error\n{source}")]
   Io { source: io::Error },

   #[error("Could not open socket on address: {value}\n{source}")]
   BadAddress {
      value: SocketAddr,
      source: io::Error,
   },

   #[error("Could not start the site server\n{source}")]
   ServeStart { source: io::Error },

   #[error("Error while serving the site\n{source}")]
   Serve { source: JoinError },

   #[error("Runtime error\n{source}")]
   Tokio {
      #[from]
      source: JoinError,
   },

   #[error("Building watcher\n{source}")]
   Watcher {
      #[from]
      source: notify::Error,
   },

   #[error(
      "Debouncing changes from the file system:\n{}",
      .0.iter()
         .map(|reason| format!("{reason}"))
         .collect::<Vec<_>>()
         .join("\n"))
   ]
   DebounceErrors(Vec<notify::Error>),

   #[error(transparent)]
   WebSocket(#[from] WebSocketError),
}

// TODO: consider moving to its own module.
#[derive(Debug, thiserror::Error)]
pub enum WebSocketError {
   #[error("Could not receive WebSocket message:\n{source}")]
   Receive { source: axum::Error },

   #[error("Unexpectedly received string WebSocket message with content:\n{0}")]
   UnexpectedText(String),

   #[error("Unexpectedly received binary WebSocket message with bytes:\n{0:?}")]
   UnexpectedBytes(Vec<u8>),
}

trait Race<T, U>: Sized {
   async fn race(self) -> Either<T, U>;
}

impl<A, B, F1, F2> Race<A, B> for (F1, F2)
where
   A: Sized,
   B: Sized,
   F1: Future<Output = A> + Unpin,
   F2: Future<Output = B> + Unpin,
{
   async fn race(self) -> Either<A, B> {
      race(self.0, self.1).await
   }
}

async fn race<A, B, F1, F2>(f1: F1, f2: F2) -> Either<A, B>
where
   F1: Future<Output = A> + Unpin,
   F2: Future<Output = B> + Unpin,
{
   match future::select(f1, f2).await {
      Either::Left((a, _f2)) => Either::Left(a),
      Either::Right((b, _f1)) => Either::Right(b),
   }
}

async fn race_all<F, I, T>(futures: I) -> T
where
   I: IntoIterator<Item = F>,
   F: Future<Output = T> + Unpin,
{
   future::select_all(futures).await.0
}
