use std::marker::PhantomData;

/// A dense arena of `T` values indexed by `Idx<T>`.
#[derive(Debug, Default)]
pub struct Arena<T> {
    data: Vec<T>,
}

/// A typed index into an `Arena<T>`.
pub struct Idx<T> {
    pub raw: u32,
    pub _phantom: PhantomData<fn() -> T>,
}

impl<T> Idx<T> {
    pub fn new(raw: u32) -> Self {
        Self {
            raw,
            _phantom: PhantomData,
        }
    }
    pub fn into_raw(self) -> u32 {
        self.raw
    }
}

impl<T> Clone for Idx<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Idx<T> {}
impl<T> PartialEq for Idx<T> {
    fn eq(&self, o: &Self) -> bool {
        self.raw == o.raw
    }
}
impl<T> Eq for Idx<T> {}
impl<T> std::hash::Hash for Idx<T> {
    fn hash<H: std::hash::Hasher>(&self, s: &mut H) {
        self.raw.hash(s)
    }
}
impl<T> std::fmt::Debug for Idx<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Idx({})", self.raw)
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn alloc(&mut self, val: T) -> Idx<T> {
        let idx = Idx::new(self.data.len() as u32);
        self.data.push(val);
        idx
    }

    pub fn get(&self, idx: Idx<T>) -> &T {
        &self.data[idx.raw as usize]
    }

    pub fn get_mut(&mut self, idx: Idx<T>) -> &mut T {
        &mut self.data[idx.raw as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = (Idx<T>, &T)> {
        self.data
            .iter()
            .enumerate()
            .map(|(i, v)| (Idx::new(i as u32), v))
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
