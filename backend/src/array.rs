use std::{
    mem,
    ops::{Index, IndexMut},
    slice::{Iter, IterMut},
};

/// A fixed size copy array.
#[derive(Debug, Clone, Copy)]
pub struct Array<T: Copy, const N: usize> {
    /// Inner array.
    ///
    /// All items in the range `0..len` are valid and initialized.
    inner: [T; N],
    /// Length of this array.
    len: usize,
}

impl<T: Copy, const N: usize> Array<T, N> {
    #[inline]
    pub const fn new() -> Self {
        Self {
            inner: [unsafe { mem::zeroed() }; N],
            len: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, value: T) {
        assert!(self.len < N);

        let index = self.len;
        self.len += 1;
        self.inner[index] = value;
    }

    #[inline]
    pub fn remove(&mut self, index: usize) {
        assert!(index < self.len);

        for i in index..self.len - 1 {
            self.inner[i] = self.inner[i + 1];
        }
        self.len -= 1;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Retrieves the slice from the array.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.inner[0..self.len]
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        self.as_slice().iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        self.inner[0..self.len].iter_mut()
    }
}

impl<T: Copy + PartialEq, const N: usize> Default for Array<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + PartialEq, const N: usize> PartialEq for Array<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T: Copy + Eq, const N: usize> Eq for Array<T, N> {}

impl<T: Copy, const N: usize> Index<usize> for Array<T, N> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        assert!(index < self.len);
        self.inner.get(index).unwrap()
    }
}

impl<T: Copy, const N: usize> IndexMut<usize> for Array<T, N> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        assert!(index < self.len);
        self.inner.get_mut(index).unwrap()
    }
}

impl<A: Copy, const N: usize> FromIterator<A> for Array<A, N> {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        let mut array = Array::new();
        for item in iter {
            array.push(item);
        }
        array
    }
}

impl<'a, T: Copy + 'a, const N: usize> IntoIterator for &'a Array<T, N> {
    type Item = &'a T;

    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: Copy, const N: usize> IntoIterator for Array<T, N> {
    type Item = T;

    type IntoIter = IntoIter<T, N>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            array: self,
            index: 0,
        }
    }
}

pub struct IntoIter<T: Copy, const N: usize> {
    array: Array<T, N>,
    index: usize,
}

impl<T: Copy, const N: usize> Iterator for IntoIter<T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let index = advance_iter_index(&mut self.index, self.array.len)?;
        self.array.inner.get(index).copied()
    }
}

#[inline]
fn advance_iter_index(index: &mut usize, len: usize) -> Option<usize> {
    if *index >= len {
        return None;
    }
    let old_index = *index;
    *index += 1;
    Some(old_index)
}

#[cfg(test)]
mod tests {
    use super::Array;

    #[test]
    fn push() {
        let mut array = Array::<u32, 1002>::new();
        let mut vec = Vec::new();
        for i in 0..1000 {
            array.push(i);
            vec.push(i);
        }
        assert_eq!(array.len(), 1000);
        assert_eq!(array.into_iter().collect::<Vec<_>>(), vec);
    }

    #[test]
    fn into_iter() {
        let mut vec = Vec::new();
        for i in 0..1000 {
            vec.push(i);
        }
        let len = vec.len();
        let array = Array::<u32, 1002>::from_iter(vec);

        assert_eq!(len, array.len());
        for (elem, i) in array.into_iter().zip(0..1000) {
            assert_eq!(elem, i);
        }

        let mut vec = Vec::new();
        for i in 333..555 {
            vec.push(i);
        }
        let len = vec.len();
        let array = Array::<u32, 1002>::from_iter(vec);

        assert_eq!(len, array.len());
        for (elem, i) in array.into_iter().zip(333..555) {
            assert_eq!(elem, i);
        }
    }

    #[test]
    fn remove() {
        let mut array = Array::<u32, 10>::new();
        for i in 0..5 {
            array.push(i);
        }

        // Array should contain [0, 1, 2, 3, 4]
        assert_eq!(array.len(), 5);

        // Remove item at index 2 (value 2)
        array.remove(2);

        // Array should now contain [0, 1, 3, 4]
        assert_eq!(array.len(), 4);
        assert_eq!(array[0], 0);
        assert_eq!(array[1], 1);
        assert_eq!(array[2], 3);
        assert_eq!(array[3], 4);
    }

    #[test]
    fn iter() {
        let mut vec = Vec::new();
        for i in 0..1000 {
            vec.push(i);
        }
        let len = vec.len();
        let array = Array::<u32, 1002>::from_iter(vec);

        assert_eq!(len, array.len());
        for (elem, i) in array.iter().zip(0..1000) {
            assert_eq!(elem, &i);
        }

        let mut vec = Vec::new();
        for i in 333..555 {
            vec.push(i);
        }
        let len = vec.len();
        let array = Array::<u32, 1002>::from_iter(vec);

        assert_eq!(len, array.len());
        for (elem, i) in array.iter().zip(333..555) {
            assert_eq!(elem, &i);
        }
    }

    #[test]
    fn as_slice() {
        let mut array = Array::<u32, 10>::new();
        for i in 0..4 {
            array.push(i);
        }

        let slice = array.as_slice();
        let expected = vec![0, 1, 2, 3];

        assert_eq!(slice.len(), 4);
        assert_eq!(slice, expected.as_slice());
    }
}
