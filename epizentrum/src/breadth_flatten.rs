use std::collections::VecDeque;

pub struct BreadthFlatten<I, T>
where
    I: Iterator<Item = T>,
    T: Sized,
{
    iterators: VecDeque<I>,
}

impl<I, T> Iterator for BreadthFlatten<I, T>
where
    I: Iterator<Item = T>,
    T: Sized,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(mut iter) = self.iterators.pop_front() {
            if let Some(item) = iter.next() {
                self.iterators.push_back(iter);
                return Some(item);
            }
        }

        None
    }
}

impl<I, T> BreadthFlatten<I, T>
where
    I: Iterator<Item = T>,
    T: Sized,
{
    pub fn new(iterators: impl Iterator<Item = I>) -> Self {
        Self {
            iterators: iterators.collect(),
        }
    }
}
