//! An event sink that keeps every event, for tests to look at.

use std::sync::{Arc, Mutex};

use orkestr8r::events::{EventSink, OrkEvent};

#[derive(Clone, Default)]
pub struct CollectSink(Arc<Mutex<Vec<OrkEvent>>>);

impl CollectSink {
    pub fn events(&self) -> Vec<OrkEvent> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for CollectSink {
    fn emit(&self, event: &OrkEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}
