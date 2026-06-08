/// Fixed-capacity circular buffer of `f64` samples shared by all indicators.
///
/// Samples are stored in insertion order; iteration is oldest-first.
#[derive(Clone, Debug)]
pub struct Window {
    buf: Vec<f64>,
    /// Index of the slot that will receive the *next* push.
    head: usize,
    len: usize,
}

impl Window {
    /// Create an empty window with the given capacity.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "window capacity must be positive");
        Self {
            buf: vec![0.0; capacity],
            head: 0,
            len: 0,
        }
    }

    /// Push a sample, evicting the oldest when the buffer is full.
    pub fn push(&mut self, v: f64) {
        self.buf[self.head] = v;
        self.head = (self.head + 1) % self.buf.len();
        if self.len < self.buf.len() {
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn is_full(&self) -> bool {
        self.len == self.buf.len()
    }

    /// The most recently pushed value, or `None` if the window is empty.
    pub fn newest(&self) -> Option<f64> {
        if self.len == 0 {
            return None;
        }
        // head points one past the newest slot.
        let idx = (self.head + self.buf.len() - 1) % self.buf.len();
        Some(self.buf[idx])
    }

    /// Iterate from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = f64> + '_ {
        let start = if self.len < self.buf.len() {
            0
        } else {
            self.head
        };
        let cap = self.buf.len();
        (0..self.len).map(move |i| self.buf[(start + i) % cap])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_iterate_in_order() {
        let mut w = Window::new(3);
        w.push(1.0);
        w.push(2.0);
        w.push(3.0);
        let v: Vec<f64> = w.iter().collect();
        assert_eq!(v, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn eviction_preserves_newest() {
        let mut w = Window::new(3);
        for i in 1..=5 {
            w.push(i as f64);
        }
        // After 5 pushes into capacity-3: holds [3, 4, 5]
        let v: Vec<f64> = w.iter().collect();
        assert_eq!(v, [3.0, 4.0, 5.0]);
        assert_eq!(w.newest(), Some(5.0));
    }

    #[test]
    fn empty_window_newest_is_none() {
        let w = Window::new(5);
        assert!(w.newest().is_none());
        assert!(w.is_empty());
    }
}
