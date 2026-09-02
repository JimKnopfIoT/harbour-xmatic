//! Public directory search. `RoomDirectorySearch` must not run `search` and
//! `next_page` at once, so one task owns it; results stream as `directory.diff`.

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::room_directory_search::{RoomDescription, RoomDirectorySearch};
use matrix_sdk::ruma::OwnedServerName;
use matrix_sdk::Client;
use matrix_sdk_ui::eyeball_im::VectorDiff;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::protocol::event;
use crate::runtime::Sink;
use crate::text::strip_bidi;

/// How many rooms one page of results carries.
const PAGE_SIZE: u32 = 20;

pub enum Order {
    /// A pattern plus the directory server to ask; `None` is the own
    /// homeserver, anything else goes out over federation.
    Search(String, Option<OwnedServerName>),
    More,
}

fn summarize(room: &RoomDescription) -> Value {
    json!({
        "id": room.room_id.as_str(),
        "name": strip_bidi(room.name.as_deref().unwrap_or_default()),
        "topic": strip_bidi(room.topic.as_deref().unwrap_or_default()),
        "alias": room
            .alias
            .as_ref()
            .map(|alias| alias.to_string())
            .unwrap_or_default(),
        "members": room.joined_members,
        "avatar": room.avatar_url.as_ref().map(|url| url.to_string()),
        "canJoin": matches!(room.join_rule, matrix_sdk::ruma::room::JoinRuleKind::Public),
    })
}

fn encode(diff: &VectorDiff<RoomDescription>) -> Value {
    crate::protocol::encode_diff(diff, summarize)
}

/// Starts the directory task. Orders go in through the returned sender; the
/// task ends when the sender is dropped.
pub fn start(client: Client, sink: Arc<Sink>) -> (JoinHandle<()>, mpsc::UnboundedSender<Order>) {
    let (orders, mut order_updates) = mpsc::unbounded_channel::<Order>();

    let task = tokio::spawn(async move {
        let mut search = RoomDirectorySearch::new(client);
        // Present but inert until the first search replaces it.
        let (_, stream) = search.results();
        let mut stream = Box::pin(stream);

        loop {
            tokio::select! {
                order = order_updates.recv() => {
                    let Some(order) = order else { break };
                    let outcome = match order {
                        Order::Search(pattern, server) => {
                            let trimmed = pattern.trim();
                            let filter = if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed.to_owned())
                            };
                            let result = search.search(filter, PAGE_SIZE, server).await;
                            // A search replaces the result vector, so the old
                            // stream is stale — resubscribe and reset the model.
                            let (initial, fresh) = search.results();
                            stream = Box::pin(fresh);
                            sink.emit(event(
                                "directory.diff",
                                json!({ "ops": [{
                                    "op": "reset",
                                    "values": initial.iter().map(summarize).collect::<Vec<_>>(),
                                }] }),
                            ));
                            result
                        }
                        Order::More => search.next_page().await,
                    };
                    // `atEnd` says whether there is more, never whether this attempt worked: the
                    // SDK keeps its `since` token, and calling a failure the end killed the search.
                    let error = outcome.err().map(|error| crate::text::scrub_ids(&format!("{error}")));
                    sink.emit(event(
                        "directory.state",
                        json!({ "atEnd": search.is_at_last_page(), "error": error }),
                    ));
                }
                diffs = stream.next() => {
                    let Some(diffs) = diffs else { break };
                    let ops: Vec<Value> = diffs.iter().map(encode).collect();
                    sink.emit(event("directory.diff", json!({ "ops": ops })));
                }
            }
        }
    });

    (task, orders)
}
