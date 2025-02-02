use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::mc::dependency::DependencyResolver;
use crate::mc::events::{McEvent, McEventId};

/// Stores pending events and provides a convenient interface for working with them.  
#[derive(Default, Clone, Hash, Eq, PartialEq, Debug)]
pub struct PendingEvents {
    events: BTreeMap<McEventId, McEvent>,
    timer_mapping: BTreeMap<(String, String), usize>,
    available_events: BTreeSet<McEventId>,
    directives: BTreeSet<McEventId>,
    resolver: DependencyResolver,
    id_counter: McEventId,
}

impl PendingEvents {
    /// Creates a new empty PendingEvents instance.
    pub fn new() -> Self {
        PendingEvents {
            events: BTreeMap::default(),
            timer_mapping: BTreeMap::default(),
            available_events: BTreeSet::default(),
            directives: BTreeSet::default(),
            resolver: DependencyResolver::default(),
            id_counter: 0,
        }
    }

    /// Stores the passed event and returns id assigned to it.
    pub fn push(&mut self, event: McEvent) -> McEventId {
        let id = self.id_counter;
        self.id_counter += 1;
        self.push_with_fixed_id(event, id)
    }

    /// Stores the passed event under the specified id (should not already exist).
    pub(crate) fn push_with_fixed_id(&mut self, event: McEvent, id: McEventId) -> McEventId {
        assert!(!self.events.contains_key(&id), "event with such id already exists");
        match &event {
            McEvent::MessageReceived { msg, src, dest, .. } => {
                if self.resolver.add_message(msg.clone(), src.clone(), dest.clone(), id) {
                    self.available_events.insert(id);
                }
            }
            McEvent::TimerFired {
                proc,
                timer_delay,
                timer,
            } => {
                self.timer_mapping.insert((proc.clone(), timer.clone()), id);
                if self.resolver.add_timer(proc.clone(), *timer_delay, id) {
                    self.available_events.insert(id);
                }
            }
            McEvent::TimerCancelled { .. } => {
                self.directives.insert(id);
            }
            McEvent::MessageDropped { .. } => {
                self.directives.insert(id);
            }
        };
        self.events.insert(id, event);
        id
    }

    /// Returns event by its id.
    pub fn get(&self, id: McEventId) -> Option<&McEvent> {
        self.events.get(&id)
    }

    /// Returns currently available events, i.e. not blocked by other events (see DependencyResolver).
    pub fn available_events(&self) -> BTreeSet<McEventId> {
        if let Some(directive) = self.directives.iter().next() {
            BTreeSet::from_iter(vec![*directive])
        } else {
            self.available_events.clone()
        }
    }

    /// Returns the number of currently available events
    pub fn available_events_num(&self) -> usize {
        if !self.directives.is_empty() {
            return 1;
        }
        self.available_events.len()
    }

    /// Cancels given timer and recalculates available events.
    pub fn cancel_timer(&mut self, proc: String, timer: String) {
        let id = self.timer_mapping.remove(&(proc, timer));
        if let Some(id) = id {
            self.pop(id);
        }
    }

    /// Removes available event by its id.
    pub fn pop(&mut self, event_id: McEventId) -> McEvent {
        let result = self.events.remove(&event_id).unwrap();
        self.directives.remove(&event_id);
        self.available_events.remove(&event_id);
        if let McEvent::TimerFired { .. } = result {
            let unblocked_events = self.resolver.remove_timer(event_id);
            self.available_events.extend(unblocked_events);
        }
        if let McEvent::MessageReceived { msg, src, dest, .. } = result.clone() {
            if let Some(unblocked_event) = self.resolver.remove_message(msg, src, dest) {
                self.available_events.insert(unblocked_event);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use rand::prelude::IteratorRandom;

    use crate::mc::events::{McEvent, McTime};
    use crate::mc::pending_events::PendingEvents;

    #[test]
    fn test_mc_time() {
        let a = McTime::from(0.0);
        let b = McTime::from(0.0);
        assert!(b <= a);
        assert!(a <= b);
        assert_eq!(a, b);
    }

    #[test]
    fn test_dependency_resolver_simple() {
        let mut pending_events = PendingEvents::new();
        let mut sequence = Vec::new();
        let mut rev_id = vec![0; 9];
        for node_id in 0..3 {
            let times: Vec<u64> = (0..3).collect();
            for event_time in times {
                let event = McEvent::TimerFired {
                    proc: node_id.to_string(),
                    timer: format!("{}", event_time),
                    timer_delay: McTime::from(event_time as f64),
                };
                rev_id[pending_events.push(event)] = event_time * 3 + node_id;
            }
        }
        println!("{:?}", rev_id);
        while let Some(id) = pending_events.available_events().iter().choose(&mut rand::thread_rng()) {
            let id = *id;
            sequence.push(rev_id[id]);
            pending_events.pop(id);
        }
        println!("{:?}", sequence);
        assert_eq!(sequence.len(), 9);
        let mut timers = vec![0, 0, 0];
        for event_id in sequence {
            let time = event_id / 3;
            let node = event_id % 3;
            assert_eq!(timers[node as usize], time);
            timers[node as usize] += 1;
        }
    }

    #[test]
    fn test_dependency_resolver_pop() {
        let mut pending_events = PendingEvents::new();
        let mut sequence = Vec::new();
        let mut rev_id = vec![0; 12];

        for node_id in 0..3 {
            let times: Vec<u64> = (0..3).collect();
            for event_time in times {
                let event = McEvent::TimerFired {
                    proc: node_id.to_string(),
                    timer: format!("{}", event_time),
                    timer_delay: McTime::from(1.0 + event_time as f64),
                };
                rev_id[pending_events.push(event)] = event_time * 3 + node_id;
            }
        }

        // remove 7 events such that every process had at least one timer fired
        // possible timer states after this:
        // - no timers
        // - one timer with delay 3
        // - two timers with delays 2 and 3
        for _ in 0..7 {
            let id = *pending_events
                .available_events()
                .iter()
                .choose(&mut rand::thread_rng())
                .unwrap();
            sequence.push(rev_id[id]);
            pending_events.pop(id);
        }

        // add one more timer to each process
        // if new timer delay is 3 or more it should be blocked by all other remaining timers if any
        // if new timer delay is less than 3, say 2.1, then it could "overtake" some of initial timers
        // (this may sound counter-intuitive since initial timers were set "at one moment" in this test,
        // however currently dependency resolver is implemented for general case when timers can be set
        // at different moments, while the optimization for timers set at one moment is not implemented)
        for node_id in 0..3 {
            let event = McEvent::TimerFired {
                proc: node_id.to_string(),
                timer: format!("{}", node_id),
                timer_delay: McTime::from(3.),
            };
            rev_id[pending_events.push(event)] = 9 + node_id;
        }
        while let Some(id) = pending_events.available_events().iter().choose(&mut rand::thread_rng()) {
            let id = *id;
            sequence.push(rev_id[id]);
            pending_events.pop(id);
        }
        println!("{:?}", sequence);
        assert_eq!(sequence.len(), 12);
        let mut timers = vec![0, 0, 0];
        for event_id in sequence {
            let time = event_id / 3;
            let node = event_id % 3;
            assert_eq!(timers[node as usize], time);
            timers[node as usize] += 1;
        }
    }
}
