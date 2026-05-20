//! Tiny fixture exercising spec §2.4 levels 17 / 19. Two local traits, a
//! local type implementing one of them, and a `Display` impl on a local
//! type (foreign-trait + local-type — the orphan-rule classic).

use std::fmt;

pub trait Describe {
    fn describe(&self) -> String;
}

pub trait Reverse {
    fn reverse(self) -> Self;
}

pub struct Counter {
    pub value: u32,
}

impl Counter {
    pub fn new(value: u32) -> Self {
        Self { value }
    }
}

impl Describe for Counter {
    fn describe(&self) -> String {
        format!("counter at {}", self.value)
    }
}

impl Reverse for Counter {
    fn reverse(mut self) -> Self {
        self.value = u32::MAX - self.value;
        self
    }
}

impl fmt::Display for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Counter({})", self.value)
    }
}
