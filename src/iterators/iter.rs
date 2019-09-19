#[cfg(feature = "parallel")]
use super::ParBuf;
use super::{AbstractMut, IntoAbstract, IntoIter, IteratorWithId};
use crate::entity::Key;
use crate::sparse_array::Pack;
#[cfg(feature = "parallel")]
use rayon::iter::plumbing::{
    bridge, bridge_unindexed, Consumer, Folder, Producer, ProducerCallback, UnindexedConsumer,
    UnindexedProducer,
};
#[cfg(feature = "parallel")]
use rayon::iter::{IndexedParallelIterator, ParallelIterator};

impl<T: IntoAbstract> IntoIter for T {
    type IntoIter = Iter1<Self>;
    #[cfg(feature = "parallel")]
    type IntoParIter = ParIter1<Self>;
    fn iter(self) -> Self::IntoIter {
        match &self.pack_info().pack {
            Pack::Update(_) => {
                let end = self.len().unwrap_or(0);
                // last_id is never read
                Iter1::Update(Update1 {
                    end,
                    data: self.into_abstract(),
                    current: 0,
                    last_id: Key::dead(),
                })
            }
            _ => Iter1::Tight(Tight1 {
                end: self.len().unwrap_or(0),
                data: self.into_abstract(),
                current: 0,
                pred: |_| true,
            }),
        }
    }
    #[cfg(feature = "parallel")]
    fn par_iter(self) -> Self::IntoParIter {
        match self.iter() {
            Iter1::Tight(iter) => ParIter1::Tight(ParTight1(iter)),
            Iter1::Update(iter) => ParIter1::Update(ParUpdate1(iter)),
        }
    }
}

impl<T: IntoIter> IntoIter for (T,) {
    type IntoIter = T::IntoIter;
    #[cfg(feature = "parallel")]
    type IntoParIter = T::IntoParIter;
    fn iter(self) -> Self::IntoIter {
        self.0.iter()
    }
    #[cfg(feature = "parallel")]
    fn par_iter(self) -> Self::IntoParIter {
        self.0.par_iter()
    }
}

pub enum Iter1<T: IntoAbstract> {
    Tight(Tight1<T>),
    Update(Update1<T>),
}

impl<T: IntoAbstract> Iter1<T> {
    /// Tries to transform the iterator into a chunk iterator, returning `size` items at a time.
    /// If the component is packed with update pack the iterator is returned.
    ///
    /// Chunk will return a smaller slice at the end if `size` does not divide exactly the length.
    pub fn into_chunk(self, size: usize) -> Result<Chunk1<T>, Self> {
        match self {
            Iter1::Tight(iter) => Ok(iter.into_chunk(size)),
            Iter1::Update(_) => Err(self),
        }
    }
    /// Tries to transform the iterator into a chunk exact iterator, returning `size` items at a time.
    /// If the component is packed with update pack the iterator is returned.
    ///
    /// ChunkExact will always return a slice with the same length.
    ///
    /// To get the remaining items (if any) use the `remainder` method.
    pub fn into_chunk_exact(self, size: usize) -> Result<ChunkExact1<T>, Self> {
        match self {
            Iter1::Tight(iter) => Ok(iter.into_chunk_exact(size)),
            Iter1::Update(_) => Err(self),
        }
    }
    pub fn filtered<P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool>(
        self,
        pred: P,
    ) -> Filter1<T, P> {
        match self {
            Iter1::Tight(iter) => Filter1::Tight(iter.filtered(pred)),
            Iter1::Update(iter) => Filter1::Update(iter.filtered(pred)),
        }
    }
}

impl<T: IntoAbstract> Iterator for Iter1<T> {
    type Item = <T::AbsView as AbstractMut>::Out;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Iter1::Tight(iter) => iter.next(),
            Iter1::Update(iter) => iter.next(),
        }
    }
    fn for_each<F>(self, f: F)
    where
        F: FnMut(Self::Item),
    {
        match self {
            Iter1::Tight(iter) => iter.for_each(f),
            Iter1::Update(iter) => iter.for_each(f),
        }
    }
    fn filter<P>(self, _: P) -> std::iter::Filter<Self, P>
    where
        P: FnMut(&Self::Item) -> bool,
    {
        panic!("use filtered instead");
    }
}

impl<T: IntoAbstract> IteratorWithId for Iter1<T> {
    fn last_id(&self) -> Key {
        match self {
            Iter1::Tight(iter) => iter.last_id(),
            Iter1::Update(iter) => iter.last_id(),
        }
    }
}

#[cfg(feature = "parallel")]
pub enum ParIter1<T: IntoAbstract> {
    Tight(ParTight1<T>),
    Update(ParUpdate1<T>),
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> ParIter1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    pub fn filtered<F: Fn(&<T::AbsView as AbstractMut>::Out) -> bool + Send + Sync>(
        self,
        pred: F,
    ) -> ParFilter1<T, F> {
        match self {
            ParIter1::Tight(iter) => ParFilter1::Tight(iter.filtered(pred)),
            ParIter1::Update(iter) => ParFilter1::Update(iter.filtered(pred)),
        }
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> ParallelIterator for ParIter1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        bridge(self, consumer)
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> IndexedParallelIterator for ParIter1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    fn len(&self) -> usize {
        match self {
            ParIter1::Tight(iter) => iter.len(),
            ParIter1::Update(iter) => iter.len(),
        }
    }
    fn drive<C: Consumer<Self::Item>>(self, consumer: C) -> C::Result {
        bridge(self, consumer)
    }
    fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
        match self {
            ParIter1::Tight(iter) => iter.with_producer(callback),
            ParIter1::Update(iter) => iter.with_producer(callback),
        }
    }
}

/// Tight iterator over a single component.
pub struct Tight1<T: IntoAbstract> {
    data: T::AbsView,
    current: usize,
    end: usize,
    pred: fn(&<T::AbsView as AbstractMut>::Out) -> bool,
}

impl<T: IntoAbstract> Tight1<T> {
    /// Transform the iterator into a chunk iterator, returning `size` items at a time.
    ///
    /// Chunk will return a smaller slice at the end if `size` does not divide exactly the length.
    pub fn into_chunk(self, size: usize) -> Chunk1<T> {
        Chunk1 {
            data: self.data,
            current: self.current,
            end: self.end,
            step: size,
        }
    }
    /// Transform the iterator into a chunk exact iterator, returning `size` items at a time.
    ///
    /// ChunkExact will always return a slice with the same length.
    ///
    /// To get the remaining items (if any) use the `remainder` method.
    pub fn into_chunk_exact(self, size: usize) -> ChunkExact1<T> {
        ChunkExact1 {
            data: self.data,
            current: self.current,
            end: self.end,
            step: size,
        }
    }
    pub fn filtered<P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool>(
        self,
        pred: P,
    ) -> std::iter::Filter<Self, P> {
        self.filter(pred)
    }
}

impl<T: IntoAbstract> Iterator for Tight1<T> {
    type Item = <T::AbsView as AbstractMut>::Out;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current;
        if current < self.end {
            self.current += 1;
            let data = unsafe { self.data.get_data(current) };
            // SAFE the index is valid and the iterator can only be created where the lifetime is valid
            Some(data)
        } else {
            None
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

impl<T: IntoAbstract> DoubleEndedIterator for Tight1<T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.end > self.current {
            self.end -= 1;
            // SAFE the index is valid and the iterator can only be created where the lifetime is valid
            Some(unsafe { self.data.get_data(self.end) })
        } else {
            None
        }
    }
}

impl<T: IntoAbstract> ExactSizeIterator for Tight1<T> {
    fn len(&self) -> usize {
        self.end - self.current
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> Producer for Tight1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    type IntoIter = Self;
    fn into_iter(self) -> Self::IntoIter {
        self
    }
    fn split_at(mut self, index: usize) -> (Self, Self) {
        let clone = Tight1 {
            data: self.data.clone(),
            current: self.current + index,
            end: self.end,
            pred: self.pred,
        };
        self.end = clone.current;
        (self, clone)
    }
}

impl<T: IntoAbstract> IteratorWithId for Tight1<T> {
    fn last_id(&self) -> Key {
        unsafe { self.data.id_at(self.current - 1) }
    }
}

/// Parallel iterator over a single component.
#[cfg(feature = "parallel")]
pub struct ParTight1<T: IntoAbstract>(Tight1<T>);

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> ParTight1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    pub fn filtered<F: Fn(&<T::AbsView as AbstractMut>::Out) -> bool + Send + Sync>(
        self,
        pred: F,
    ) -> rayon::iter::Filter<Self, F> {
        self.filter(pred)
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> ParallelIterator for ParTight1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        bridge(self, consumer)
    }
    fn opt_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> IndexedParallelIterator for ParTight1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    fn len(&self) -> usize {
        self.0.end - self.0.current
    }
    fn drive<C: Consumer<Self::Item>>(self, consumer: C) -> C::Result {
        bridge(self, consumer)
    }
    fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
        callback.callback(self.0)
    }
}

/// Chunk iterator over a single component.
///
/// Returns `size` long slices and not single elements.
///
/// The last chunk's length will be smaller than `size` if `size` does not divide the iterator's length perfectly.
pub struct Chunk1<T: IntoAbstract> {
    data: T::AbsView,
    current: usize,
    end: usize,
    step: usize,
}

impl<T: IntoAbstract> Iterator for Chunk1<T> {
    type Item = <T::AbsView as AbstractMut>::Slice;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current;
        if current + self.step < self.end {
            self.current += self.step;
            Some(unsafe { self.data.get_data_slice(current..(current + self.step)) })
        } else if current < self.end {
            self.current = self.end;
            Some(unsafe { self.data.get_data_slice(current..self.end) })
        } else {
            None
        }
    }
}

/// Chunk exact iterator over a single component.
///
/// Returns `size` long slices and not single elements.
///
/// The slices length will always by the same. To get the remaining elements (if any) use [remainder].
///
/// [remainder]: struct.ChunkExact1.html#method.remainder
pub struct ChunkExact1<T: IntoAbstract> {
    data: T::AbsView,
    current: usize,
    end: usize,
    step: usize,
}

impl<T: IntoAbstract> ChunkExact1<T> {
    /// Returns the items at the end of the slice.
    ///
    /// Will always return a slice smaller than `size`.
    pub fn remainder(&mut self) -> <T::AbsView as AbstractMut>::Slice {
        let remainder = std::cmp::min(self.end - self.current, self.end % self.step);
        let old_end = self.end;
        self.end -= remainder;
        unsafe { self.data.get_data_slice(self.end..old_end) }
    }
}

impl<T: IntoAbstract> Iterator for ChunkExact1<T> {
    type Item = <T::AbsView as AbstractMut>::Slice;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current;
        if current + self.step <= self.end {
            self.current += self.step;
            Some(unsafe { self.data.get_data_slice(current..self.current) })
        } else {
            None
        }
    }
}

pub enum Filter1<
    T: IntoAbstract,
    P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool,
> {
    Tight(std::iter::Filter<Tight1<T>, P>),
    Update(UpdateFilter1<T, P>),
}

impl<T: IntoAbstract, P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool>
    Iterator for Filter1<T, P>
{
    type Item = <Tight1<T> as Iterator>::Item;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Filter1::Tight(iter) => iter.next(),
            Filter1::Update(iter) => iter.next(),
        }
    }
}

#[cfg(feature = "parallel")]
pub enum ParFilter1<T: IntoAbstract, P>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    Tight(rayon::iter::Filter<ParTight1<T>, P>),
    Update(ParUpdateFilter1<T, P>),
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract, P> ParallelIterator for ParFilter1<T, P>
where
    <T::AbsView as AbstractMut>::Out: Send,
    P: Fn(&<T::AbsView as AbstractMut>::Out) -> bool + Send + Sync,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        match self {
            ParFilter1::Tight(iter) => iter.drive_unindexed(consumer),
            ParFilter1::Update(iter) => iter.drive_unindexed(consumer),
        }
    }
}

pub struct Update1<T: IntoAbstract> {
    data: T::AbsView,
    current: usize,
    end: usize,
    last_id: Key,
}

impl<T: IntoAbstract> Update1<T> {
    pub fn filtered<P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool>(
        self,
        pred: P,
    ) -> UpdateFilter1<T, P> {
        UpdateFilter1 {
            data: self.data,
            current: self.current,
            end: self.end,
            pred,
        }
    }
}

impl<T: IntoAbstract> Iterator for Update1<T> {
    type Item = <T::AbsView as AbstractMut>::Out;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current;
        if current < self.end {
            self.current += 1;
            // SAFE the index is valid and the iterator can only be created where the lifetime is valid
            Some(unsafe { self.data.mark_modified(current) })
        } else {
            None
        }
    }
    fn filter<P>(self, _: P) -> std::iter::Filter<Self, P>
    where
        P: FnMut(&Self::Item) -> bool,
    {
        panic!("use filtered instead");
    }
}

impl<T: IntoAbstract> IteratorWithId for Update1<T> {
    fn last_id(&self) -> Key {
        self.last_id
    }
}

#[cfg(feature = "parallel")]
pub struct ParUpdate1<T: IntoAbstract>(Update1<T>);

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> ParUpdate1<T> {
    pub fn filtered<P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool>(
        self,
        pred: P,
    ) -> ParUpdateFilter1<T, P> {
        ParUpdateFilter1 { iter: self, pred }
    }
}

#[cfg(feature = "parallel")]
pub struct InnerParUpdate1<'a, T: IntoAbstract> {
    iter: Update1<T>,
    indices: &'a ParBuf<usize>,
}

#[cfg(feature = "parallel")]
impl<'a, T: IntoAbstract> Producer for InnerParUpdate1<'a, T> {
    type Item = <T::AbsView as AbstractMut>::Out;
    type IntoIter = ParSeqUpdate1<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        ParSeqUpdate1 {
            indices: self.indices,
            data: self.iter.data,
            current: self.iter.current,
            end: self.iter.end,
        }
    }
    fn split_at(mut self, index: usize) -> (Self, Self) {
        let clone = InnerParUpdate1 {
            // last_id is never read
            iter: Update1 {
                data: self.iter.data.clone(),
                current: self.iter.current + index,
                end: self.iter.end,
                last_id: Key::dead(),
            },
            indices: self.indices,
        };
        self.iter.end = clone.iter.current;
        (self, clone)
    }
}

#[cfg(feature = "parallel")]
pub struct ParSeqUpdate1<'a, T: IntoAbstract> {
    indices: &'a ParBuf<usize>,
    data: T::AbsView,
    current: usize,
    end: usize,
}

#[cfg(feature = "parallel")]
impl<'a, T: IntoAbstract> Iterator for ParSeqUpdate1<'a, T> {
    type Item = <T::AbsView as AbstractMut>::Out;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current;
        if current < self.end {
            self.current += 1;
            self.indices.push(current);
            // SAFE the index is valid and the iterator can only be created where the lifetime is valid
            Some(unsafe { self.data.get_data(current) })
        } else {
            None
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

#[cfg(feature = "parallel")]
impl<'a, T: IntoAbstract> DoubleEndedIterator for ParSeqUpdate1<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.end > self.current {
            self.end -= 1;
            self.indices.push(self.end);
            // SAFE the index is valid and the iterator can only be created where the lifetime is valid
            Some(unsafe { self.data.get_data(self.end) })
        } else {
            None
        }
    }
}

#[cfg(feature = "parallel")]
impl<'a, T: IntoAbstract> ExactSizeIterator for ParSeqUpdate1<'a, T> {
    fn len(&self) -> usize {
        self.end - self.current
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> ParallelIterator for ParUpdate1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        bridge(self, consumer)
    }
    fn opt_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract> IndexedParallelIterator for ParUpdate1<T>
where
    <T::AbsView as AbstractMut>::Out: Send,
{
    fn len(&self) -> usize {
        self.0.end - self.0.current
    }
    fn drive<C: Consumer<Self::Item>>(self, consumer: C) -> C::Result {
        bridge(self, consumer)
    }
    fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
        use std::sync::atomic::Ordering;

        let mut data = self.0.data.clone();
        let len = self.0.end - self.0.current;
        let indices = ParBuf::new(len);

        let inner = InnerParUpdate1 {
            iter: self.0,
            indices: &indices,
        };

        let result = callback.callback(inner);
        let slice = unsafe {
            std::slice::from_raw_parts_mut(indices.buf, indices.len.load(Ordering::Relaxed))
        };
        slice.sort();
        for &mut index in slice {
            unsafe { data.mark_modified(index) };
        }
        result
    }
}

#[cfg(feature = "parallel")]
pub struct ParUpdateFilter1<T: IntoAbstract, P> {
    iter: ParUpdate1<T>,
    pred: P,
}

#[cfg(feature = "parallel")]
impl<T: IntoAbstract, P> ParallelIterator for ParUpdateFilter1<T, P>
where
    P: Fn(&<T::AbsView as AbstractMut>::Out) -> bool + Send + Sync,
    <T::AbsView as AbstractMut>::Out: Send,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        use std::sync::atomic::Ordering;

        let mut data = self.iter.0.data.clone();
        let len = self.iter.0.end - self.iter.0.current;
        let indices = ParBuf::new(len);

        let producer = UpdateFilterProducer1 {
            inner: InnerParUpdate1 {
                iter: self.iter.0,
                indices: &indices,
            },
            pred: &self.pred,
        };

        let result = bridge_unindexed(producer, consumer);

        let slice = unsafe {
            std::slice::from_raw_parts_mut(indices.buf, indices.len.load(Ordering::Relaxed))
        };
        slice.sort();
        for &mut index in slice {
            unsafe { data.mark_modified(index) };
        }

        result
    }
}

#[cfg(feature = "parallel")]
pub struct UpdateFilterProducer1<'a, T: IntoAbstract, P> {
    inner: InnerParUpdate1<'a, T>,
    pred: &'a P,
}

#[cfg(feature = "parallel")]
impl<'a, T: IntoAbstract, P> UnindexedProducer for UpdateFilterProducer1<'a, T, P>
where
    P: Fn(&<T::AbsView as AbstractMut>::Out) -> bool + Send + Sync,
{
    type Item = <T::AbsView as AbstractMut>::Out;
    fn split(mut self) -> (Self, Option<Self>) {
        let len = self.inner.iter.end - self.inner.iter.current;
        if len >= 2 {
            let (left, right) = self.inner.split_at(len / 2);
            self.inner = left;
            let clone = UpdateFilterProducer1 {
                inner: right,
                pred: self.pred,
            };
            (self, Some(clone))
        } else {
            (self, None)
        }
    }
    fn fold_with<F>(mut self, mut folder: F) -> F
    where
        F: Folder<Self::Item>,
    {
        for index in self.inner.iter.current..self.inner.iter.end {
            let item = unsafe { self.inner.iter.data.get_data(index) };
            if (self.pred)(&item) {
                self.inner.indices.push(index);
                folder = folder.consume(item);
            }
        }
        folder
    }
}

pub struct UpdateFilter1<
    T: IntoAbstract,
    P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool,
> {
    data: T::AbsView,
    current: usize,
    end: usize,
    pred: P,
}

impl<T: IntoAbstract, P: FnMut(&<<T as IntoAbstract>::AbsView as AbstractMut>::Out) -> bool>
    Iterator for UpdateFilter1<T, P>
{
    type Item = <Tight1<T> as Iterator>::Item;
    fn next(&mut self) -> Option<Self::Item> {
        while self.current < self.end {
            self.current += 1;
            // SAFE the index is valid and the iterator can only be created where the lifetime is valid
            if (self.pred)(unsafe { &self.data.get_data(self.current - 1) }) {
                return Some(unsafe { self.data.mark_modified(self.current - 1) });
            }
        }
        None
    }
}

pub struct WithId<I>(pub(super) I);

impl<I: IteratorWithId> Iterator for WithId<I> {
    type Item = (Key, I::Item);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|item| (self.0.last_id(), item))
    }
}

macro_rules! impl_iterators {
    (
        $number: literal
        $iter: ident
        $par_iter: ident
        $tight: ident
        $loose: ident
        $non_packed: ident
        $chunk: ident
        $chunk_exact: ident
        $par_tight: ident
        $par_loose: ident
        $par_non_packed: ident
        $filter: ident
        $update: ident
        $par_update: ident
        $inner_par_update: ident
        $par_seq_update: ident
        $(($type: ident, $index: tt))+
    ) => {
        impl<$($type: IntoAbstract),+> IntoIter for ($($type,)+) {
            type IntoIter = $iter<$($type,)+>;
            #[cfg(feature = "parallel")]
            type IntoParIter = $par_iter<$($type,)+>;
            fn iter(self) -> Self::IntoIter {
                #[derive(PartialEq, Eq)]
                enum PackIter {
                    Tight,
                    Loose,
                    Update,
                    None,
                }

                let mut type_ids = [$(self.$index.type_id()),+];
                type_ids.sort_unstable();
                let mut smallest_index = std::usize::MAX;
                let mut smallest = std::usize::MAX;
                let mut i = 0;
                let mut pack_iter = PackIter::None;

                $({
                    if pack_iter == PackIter::None || pack_iter == PackIter::Update {
                        match &self.$index.pack_info().pack {
                            Pack::Tight(pack) => {
                                if let Ok(types) = pack.check_types(&type_ids) {
                                    if types.len() == type_ids.len() {
                                        pack_iter = PackIter::Tight;
                                        smallest = pack.len;
                                    } else if pack.len < smallest {
                                        smallest = pack.len;
                                        smallest_index = i;
                                    }
                                } else if let Some(len) = self.$index.len() {
                                    if len < smallest {
                                        smallest = len;
                                        smallest_index = i;
                                    }
                                }
                            }
                            Pack::Loose(pack) => {
                                if pack.check_all_types(&type_ids).is_ok() {
                                    if pack.tight_types.len() + pack.loose_types.len() == type_ids.len() {
                                        pack_iter = PackIter::Loose;
                                        smallest = pack.len;
                                        smallest_index = i;
                                    } else if pack.len < smallest {
                                        smallest = pack.len;
                                        smallest_index = i;
                                    }
                                } else if let Some(len) = self.$index.len() {
                                    if len < smallest {
                                        smallest = len;
                                        smallest_index = i;
                                    }
                                }
                            }
                            Pack::Update(_) => {
                                pack_iter = PackIter::Update;
                                if let Some(len) = self.$index.len() {
                                    if len < smallest {
                                        smallest = len;
                                        smallest_index = i;
                                    }
                                }
                            }
                            Pack::NoPack => if let Some(len) = self.$index.len() {
                                if len < smallest {
                                    smallest = len;
                                    smallest_index = i;
                                }
                            }
                        }

                        i += 1;
                    }
                })+

                let _ = i;

                match pack_iter {
                    PackIter::Tight => {
                        $iter::Tight($tight {
                            data: ($(self.$index.into_abstract(),)+),
                            current: 0,
                            end: smallest,
                        })
                    }
                    PackIter::Loose => {
                        let mut indices = None;
                        let mut array = 0;
                        $(
                            if let Pack::Loose(_) = self.$index.pack_info().pack {
                                array |= 1 << $index;
                            }
                        )+
                        let data = ($(self.$index.into_abstract(),)+);
                        $(
                            if $index == smallest_index {
                                indices = Some(data.$index.indices());
                            }
                        )+
                        $iter::Loose($loose {
                            data,
                            current: 0,
                            end: smallest,
                            array,
                            indices: indices.unwrap(),
                        })
                    }
                    PackIter::Update => {
                        let mut indices = None;
                        let data = ($(self.$index.into_abstract(),)+);
                        // if the user is trying to iterate over Not containers only
                        if smallest == std::usize::MAX {
                            smallest = 0;
                        } else {
                            $(
                                if $index == smallest_index {
                                    indices = Some(data.$index.indices());
                                }
                            )+
                        }

                        $iter::Update($update {
                            data,
                            indices: indices.unwrap_or(std::ptr::null()),
                            current: 0,
                            end: smallest,
                            array: smallest_index,
                        })
                    }
                    PackIter::None => {
                        let mut indices = None;
                        let data = ($(self.$index.into_abstract(),)+);
                        // if the user is trying to iterate over Not containers only
                        if smallest == std::usize::MAX {
                            smallest = 0;
                        } else {
                            $(
                                if $index == smallest_index {
                                    indices = Some(data.$index.indices());
                                }
                            )+
                        }

                        $iter::NonPacked($non_packed {
                            data,
                            indices: indices.unwrap_or(std::ptr::null()),
                            current: 0,
                            end: smallest,
                            array: smallest_index,
                        })
                    }
                }
            }
            #[cfg(feature = "parallel")]
            fn par_iter(self) -> Self::IntoParIter {
                match self.iter() {
                    $iter::Tight(iter) => $par_iter::Tight($par_tight(iter)),
                    $iter::Loose(iter) => $par_iter::Loose($par_loose(iter)),
                    $iter::Update(iter) => $par_iter::Update($par_update(iter)),
                    $iter::NonPacked(iter) => $par_iter::NonPacked($par_non_packed(iter)),
                }
            }
        }

        #[doc = "Iterator over"]
        #[doc = $number]
        #[doc = "components.\n This enum allows to abstract away what kind of iterator you really get. That doesn't mean the performance will suffer, the compiler will (almost)
        always optimize it away."]
        pub enum $iter<$($type: IntoAbstract),+> {
            Tight($tight<$($type),+>),
            Loose($loose<$($type),+>),
            Update($update<$($type),+>),
            NonPacked($non_packed<$($type),+>),
        }

        impl<$($type: IntoAbstract),+> $iter<$($type),+> {
            /// Tries to transform the iterator into a chunk iterator, returning `size` items at a time.
            /// If the components are not tightly packed together the iterator is returned.
            ///
            /// Chunk will return a smaller slice at the end if `size` does not divide exactly the length.
            pub fn into_chunk(self, size: usize) -> Result<$chunk<$($type),+>, Self> {
                match self {
                    $iter::Tight(iter) => Ok(iter.into_chunk(size)),
                    $iter::Loose(_) => Err(self),
                    $iter::Update(_) => Err(self),
                    $iter::NonPacked(_) => Err(self),
                }
            }
            /// Tries to transform the iterator into a chunk exact iterator, returning `size` items at a time.
            /// If the components are not tightly packed together the iterator is returned.
            ///
            /// ChunkExact will always return a slice with the same length.
            ///
            /// To get the remaining items (if any) use the `remainder` method.
            pub fn into_chunk_exact(self, size: usize) -> Result<$chunk_exact<$($type),+>, Self> {
                match self {
                    $iter::Tight(iter) => Ok(iter.into_chunk_exact(size)),
                    $iter::Loose(_) => Err(self),
                    $iter::Update(_) => Err(self),
                    $iter::NonPacked(_) => Err(self),
                }
            }
            pub fn filtered<P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool>(self, pred: P) -> $filter<$($type,)+ P> {
                $filter {
                    iter: self,
                    pred,
                }
            }
        }

        impl<$($type: IntoAbstract),+> Iterator for $iter<$($type),+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn next(&mut self) -> Option<Self::Item> {
                match self {
                    $iter::Tight(iter) => iter.next(),
                    $iter::Loose(iter) => iter.next(),
                    $iter::Update(iter) => iter.next(),
                    $iter::NonPacked(iter) => iter.next(),
                }
            }
        }

        #[doc = "Parallel iterator over"]
        #[doc = $number]
        #[doc = "components.\n This enum allows to abstract away what kind of iterator you really get. That doesn't mean the performance will suffer, the compiler will (almost)
        always optimize it away."]
        #[cfg(feature = "parallel")]
        pub enum $par_iter<$($type: IntoAbstract),+> {
            Tight($par_tight<$($type),+>),
            Loose($par_loose<$($type),+>),
            Update($par_update<$($type),+>),
            NonPacked($par_non_packed<$($type),+>),
        }

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> ParallelIterator for $par_iter<$($type),+>
        where $(<$type::AbsView as AbstractMut>::Out: Send),+ {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn drive_unindexed<Cons>(self, consumer: Cons) -> Cons::Result
            where
                Cons: UnindexedConsumer<Self::Item>,
            {
                match self {
                    $par_iter::Tight(iter) => bridge(iter, consumer),
                    $par_iter::Loose(iter) => bridge(iter, consumer),
                    $par_iter::Update(iter) => iter.drive_unindexed(consumer),
                    $par_iter::NonPacked(iter) => iter.drive_unindexed(consumer),
                }
            }
        }

        #[doc = "Tight iterator over"]
        #[doc = $number]
        #[doc = "components.\n Tight iterators are fast but are limited to components tightly packed together."]
        pub struct $tight<$($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            current: usize,
            end: usize,
        }

        impl<$($type: IntoAbstract),+> $tight<$($type),+> {
            /// Transform the iterator into a chunk iterator, returning `size` items at a time.
            ///
            /// Chunk will return a smaller slice at the end if `size` does not divide exactly the length.
            pub fn into_chunk(self, size: usize) -> $chunk<$($type),+> {
                $chunk {
                    data: self.data,
                    current: self.current,
                    end: self.end,
                    step: size,
                }
            }
            /// Transform the iterator into a chunk exact iterator, returning `size` items at a time.
            ///
            /// ChunkExact will always return a slice with the same length.
            ///
            /// To get the remaining items (if any) use the `remainder` method.
            pub fn into_chunk_exact(self, size: usize) -> $chunk_exact<$($type),+> {
                $chunk_exact {
                    data: self.data,
                    current: self.current,
                    end: self.end,
                    step: size,
                }
            }
            pub fn filtered<P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool>(self, pred: P) -> $filter<$($type,)+ P> {
                $filter {
                    iter: $iter::Tight(self),
                    pred,
                }
            }
        }

        impl<$($type: IntoAbstract),+> Iterator for $tight<$($type),+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn next(&mut self) -> Option<Self::Item> {
                let current = self.current;
                if current < self.end {
                    self.current += 1;
                    // SAFE the index is valid and the iterator can only be created where the lifetime is valid
                    Some(unsafe { ($(self.data.$index.get_data(current),)+) })
                } else {
                    None
                }
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (self.len(), Some(self.len()))
            }
        }

        impl<$($type: IntoAbstract),+> DoubleEndedIterator for $tight<$($type),+> {
            fn next_back(&mut self) -> Option<Self::Item> {
                if self.end > self.current {
                    self.end -= 1;
                    // SAFE the index is valid and the iterator can only be created where the lifetime is valid
                    Some(unsafe { ($(self.data.$index.get_data(self.end),)+) })
                } else {
                    None
                }
            }
        }

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> Producer for $tight<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            type IntoIter = Self;
            fn into_iter(self) -> Self::IntoIter {
                self
            }
            fn split_at(mut self, index: usize) -> (Self, Self) {
                let clone = $tight {
                    data: self.data.clone(),
                    current: self.current + index,
                    end: self.end,
                };
                self.end = clone.current;
                (self, clone)
            }
        }

        impl<$($type: IntoAbstract),+> ExactSizeIterator for $tight<$($type),+> {
            fn len(&self) -> usize {
                self.end - self.current
            }
        }

        #[doc = "Parallel tight iterator over"]
        #[doc = $number]
        #[doc = "components.\n Tight iterators are fast but are limited to components tightly packed together."]
        #[cfg(feature = "parallel")]
        pub struct $par_tight<$($type: IntoAbstract),+>($tight<$($type),+>);

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> ParallelIterator for $par_tight<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn drive_unindexed<Cons>(self, consumer: Cons) -> Cons::Result
            where
                Cons: UnindexedConsumer<Self::Item>,
            {
                bridge(self, consumer)
            }
            fn opt_len(&self) -> Option<usize> {
                Some(self.len())
            }
        }

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> IndexedParallelIterator for $par_tight<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            fn len(&self) -> usize {
                self.0.end - self.0.current
            }
            fn drive<Cons: Consumer<Self::Item>>(self, consumer: Cons) -> Cons::Result {
                bridge(self, consumer)
            }
            fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
                callback.callback(self.0)
            }
        }

        #[doc = "Chunk iterator over"]
        #[doc = $number]
        #[doc = "components.\n Returns a tuple of `size` long slices and not single elements.\n The last chunk's length will be smaller than `size` if `size` does not divide the iterator's length perfectly."]
        pub struct $chunk<$($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            current: usize,
            end: usize,
            step: usize,
        }

        impl<$($type: IntoAbstract),+> Iterator for $chunk<$($type),+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Slice,)+);
            fn next(&mut self) -> Option<Self::Item> {
                let current = self.current;
                if current + self.step <= self.end {
                    self.current += self.step;
                    Some(($(unsafe { self.data.$index.get_data_slice(current..(current + self.step)) },)+))
                } else if current < self.end {
                    self.current = self.end;
                    Some(($(unsafe { self.data.$index.get_data_slice(current..self.end) },)+))
                } else {
                    None
                }
            }
        }

        #[doc = "Chunk exact iterator over"]
        #[doc = $number]
        #[doc = "components.\n Returns a tuple of `size` long slices and not single elements.\n ChunkExact will always return a slice with the same length.\n To get the remaining items (if any) use the `remainder` method."]
        pub struct $chunk_exact<$($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            current: usize,
            end: usize,
            step: usize,
        }

        impl<$($type: IntoAbstract),+> Iterator for $chunk_exact<$($type),+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Slice,)+);
            fn next(&mut self) -> Option<Self::Item> {
                let current = self.current;
                if current + self.step <= self.end {
                    self.current += self.step;
                    Some(($(unsafe { self.data.$index.get_data_slice(current..(current + self.step)) },)+))
                } else {
                    None
                }
            }
        }

        impl<$($type: IntoAbstract),+> $chunk_exact<$($type),+> {
            /// Returns the items at the end of the iterator.
            ///
            /// Will always return a slice smaller than `size`.
            pub fn remainder(&mut self) -> ($(<$type::AbsView as AbstractMut>::Slice,)+) {
                let end = self.end;
                let remainder = std::cmp::min(self.end - self.current, self.end % self.step);
                self.end -= remainder;
                ($(
                    unsafe { self.data.$index.get_data_slice((end - remainder)..end) },
                )+)
            }
        }

        #[doc = "Loose packed iterator over"]
        #[doc = $number]
        #[doc = "components.\n"]
        pub struct $loose<$($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            indices: *const Key,
            current: usize,
            end: usize,
            array: u32,
        }

        unsafe impl<$($type: IntoAbstract),+> Send for $loose<$($type),+> {}

        impl<$($type: IntoAbstract),+> $loose<$($type),+> {
            pub fn filtered<P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool>(self, pred: P) -> $filter<$($type,)+ P> {
                $filter {
                    iter: $iter::Loose(self),
                    pred,
                }
            }
        }

        impl<$($type: IntoAbstract),+> Iterator for $loose<$($type,)+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn next(&mut self) -> Option<Self::Item> {
                if self.current < self.end {
                    // SAFE at this point there are no mutable reference to sparse or dense
                    // and self.indices can't access out of bounds
                    let index = unsafe { std::ptr::read(self.indices.add(self.current)) };
                    self.current += 1;
                    let indices = ($(
                        if (self.array >> $index) & 1 != 0 {
                            self.current - 1
                        } else {
                            unsafe { self.data.$index.index_of_unchecked(index) }
                        },
                    )+);
                    Some(($({
                        unsafe { self.data.$index.get_data(indices.$index) }
                    },)+))
                } else {
                    None
                }
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (self.len(), Some(self.len()))
            }
        }

        impl<$($type: IntoAbstract),+> DoubleEndedIterator for $loose<$($type,)+> {
            fn next_back(&mut self) -> Option<Self::Item> {
                if self.end > self.current {
                    self.end -= 1;
                    // SAFE at this point there are no mutable reference to sparse or dense
                    // and self.indices can't access out of bounds
                    let index = unsafe { std::ptr::read(self.indices.add(self.end)) };
                    let indices = ($(
                        if (self.array >> $index) & 1 != 0 {
                            self.end
                        } else {
                            unsafe { self.data.$index.index_of_unchecked(index) }
                        },
                    )+);
                    Some(($({
                        unsafe { self.data.$index.get_data(indices.$index) }
                    },)+))
                } else {
                    None
                }
            }
        }

        impl<$($type: IntoAbstract),+> ExactSizeIterator for $loose<$($type),+> {
            fn len(&self) -> usize {
                self.end - self.current
            }
        }

        #[doc = "Parallel loose iterator over"]
        #[doc = $number]
        #[doc = "components.\n"]
        #[cfg(feature = "parallel")]
        pub struct $par_loose<$($type: IntoAbstract),+>($loose<$($type),+>);

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> Producer for $loose<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            type IntoIter = Self;
            fn into_iter(self) -> Self::IntoIter {
                self
            }
            fn split_at(mut self, index: usize) -> (Self, Self) {
                let clone = $loose {
                    data: self.data.clone(),
                    indices: self.indices,
                    current: self.current + index,
                    end: self.end,
                    array: self.array,
                };
                self.end = clone.current;
                (self, clone)
            }
        }

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> ParallelIterator for $par_loose<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn drive_unindexed<Cons>(self, consumer: Cons) -> Cons::Result
            where
                Cons: UnindexedConsumer<Self::Item>,
            {
                bridge(self, consumer)
            }
            fn opt_len(&self) -> Option<usize> {
                Some(self.len())
            }
        }

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> IndexedParallelIterator for $par_loose<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            fn len(&self) -> usize {
                self.0.end - self.0.current
            }
            fn drive<Cons: Consumer<Self::Item>>(self, consumer: Cons) -> Cons::Result {
                bridge(self, consumer)
            }
            fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
                callback.callback(self.0)
            }
        }

        #[doc = "Non packed iterator over"]
        #[doc = $number]
        #[doc = "components.\n"]
        pub struct $non_packed<$($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            indices: *const Key,
            current: usize,
            end: usize,
            array: usize,
        }

        unsafe impl<$($type: IntoAbstract),+> Send for $non_packed<$($type),+> {}

        impl<$($type: IntoAbstract),+> $non_packed<$($type),+> {
            #[cfg(feature = "parallel")]
            fn clone(&self) -> Self {
                $non_packed {
                    data: self.data.clone(),
                    indices: self.indices,
                    current: self.current,
                    end: self.end,
                    array: self.array,
                }
            }
            pub fn filtered<P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool>(self, pred: P) -> $filter<$($type,)+ P> {
                $filter {
                    iter: $iter::NonPacked(self),
                    pred,
                }
            }
        }

        impl<$($type: IntoAbstract),+> Iterator for $non_packed<$($type,)+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn next(&mut self) -> Option<Self::Item> {
                while self.current < self.end {
                    // SAFE at this point there are no mutable reference to sparse or dense
                    // and self.indices can't access out of bounds
                    let index = unsafe { std::ptr::read(self.indices.add(self.current)) };
                    self.current += 1;
                    let data_indices = ($(
                        if $index == self.array {
                            self.current - 1
                        } else {
                            if let Some(index) = self.data.$index.index_of(index) {
                                index
                            } else {
                                continue
                            }
                        },
                    )+);
                    unsafe {
                        return Some(($(self.data.$index.get_data(data_indices.$index),)+))
                    }
                }
                None
            }
        }

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> UnindexedProducer for $non_packed<$($type,)+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn split(mut self) -> (Self, Option<Self>) {
                let len = self.end - self.current;
                if len >= 2 {
                    let mut clone = self.clone();
                    clone.current += len / 2;
                    self.end = clone.current;
                    (self, Some(clone))
                } else {
                    (self, None)
                }
            }
            fn fold_with<Fold>(self, folder: Fold) -> Fold
            where Fold: Folder<Self::Item> {
                folder.consume_iter(self)
            }
        }

        #[doc = "Parallel non packed iterator over"]
        #[doc = $number]
        #[doc = "components.\n"]
        #[cfg(feature = "parallel")]
        pub struct $par_non_packed<$($type: IntoAbstract),+>($non_packed<$($type),+>);

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> ParallelIterator for $par_non_packed<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn drive_unindexed<Cons>(self, consumer: Cons) -> Cons::Result
            where
                Cons: UnindexedConsumer<Self::Item>,
            {
                bridge_unindexed(self.0, consumer)
            }
        }

        pub struct $filter<$($type: IntoAbstract,)+ P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool> {
            iter: $iter<$($type,)+>,
            pred: P,
        }

        impl<$($type: IntoAbstract,)+ P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool> Iterator for $filter<$($type,)+ P> {
            type Item = ($(<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+);
            fn next(&mut self) -> Option<Self::Item> {
                match &mut self.iter {
                    $iter::Tight(iter) => {
                        for item in iter {
                            if (self.pred)($(&item.$index),+) {
                                return Some(item);
                            }
                        }
                    }
                    $iter::Loose(iter) => {
                        for item in iter {
                            if (self.pred)($(&item.$index),+) {
                                return Some(item);
                            }
                        }
                    }
                    $iter::NonPacked(iter) => {
                        for item in iter {
                            if (self.pred)($(&item.$index),+) {
                                return Some(item);
                            }
                        }
                    }
                    $iter::Update(iter) => {
                        while iter.current < iter.end {
                            let index = unsafe { std::ptr::read(iter.indices.add(iter.current)) };

                            iter.current += 1;

                            let data_indices = ($(
                                if $index == iter.array {
                                    iter.current - 1
                                } else {
                                    if let Some(index) = iter.data.$index.index_of(index) {
                                        index
                                    } else {
                                        continue
                                    }
                                },
                            )+);
                            if (self.pred)($(unsafe {&iter.data.$index.get_data(data_indices.$index)}),+) {
                                return Some(($(
                                    unsafe {
                                        iter.data.$index.mark_modified(data_indices.$index)
                                    },
                                )+))
                            }
                        }
                    }
                }
                None
            }
        }

        pub struct $update<$($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            indices: *const Key,
            current: usize,
            end: usize,
            array: usize,
        }

        unsafe impl<$($type: IntoAbstract),+> Send for $update<$($type),+> {}

        impl<$($type: IntoAbstract),+> $update<$($type),+> {
            pub fn filtered<P: FnMut($(&<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+) -> bool>(self, pred: P) -> $filter<$($type,)+ P> {
                $filter {
                    iter: $iter::Update(self),
                    pred,
                }
            }
        }

        impl<$($type: IntoAbstract,)+> Iterator for $update<$($type),+> {
            type Item = ($(<<$type as IntoAbstract>::AbsView as AbstractMut>::Out),+);
            fn next(&mut self) -> Option<Self::Item> {
                while self.current < self.end {
                    // SAFE at this point there are no mutable reference to sparse or dense
                    // and self.indices can't access out of bounds
                    let index = unsafe { std::ptr::read(self.indices.add(self.current)) };
                    self.current += 1;
                    let data_indices = ($(
                        if $index == self.array {
                            self.current - 1
                        } else {
                            if let Some(index) = self.data.$index.index_of(index) {
                                index
                            } else {
                                continue
                            }
                        },
                    )+);
                    unsafe {
                        return Some(($(self.data.$index.mark_modified(data_indices.$index),)+))
                    }
                }
                None
            }
        }

        #[cfg(feature = "parallel")]
        pub struct $par_update<$($type: IntoAbstract),+>($update<$($type),+>);

        #[cfg(feature = "parallel")]
        impl<$($type: IntoAbstract),+> ParallelIterator for $par_update<$($type),+>
        where
            $(<$type::AbsView as AbstractMut>::Out: Send),+
        {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn drive_unindexed<Cons>(self, consumer: Cons) -> Cons::Result
            where
                Cons: UnindexedConsumer<Self::Item>,
            {
                use std::sync::atomic::Ordering;

                let mut data = self.0.data.clone();
                let len = self.0.end - self.0.current;
                let updated = ParBuf::new(len);

                let inner = $inner_par_update {
                    iter: self.0,
                    updated: &updated,
                };

                let result = bridge_unindexed(inner, consumer);
                let slice = unsafe {
                    std::slice::from_raw_parts_mut(updated.buf, updated.len.load(Ordering::Relaxed))
                };
                for &mut index in slice {
                    $(
                        unsafe {data.$index.mark_id_modified(index)};
                    )+
                }
                result
            }
        }

        #[cfg(feature = "parallel")]
        pub struct $inner_par_update<'a, $($type: IntoAbstract),+> {
            iter: $update<$($type),+>,
            updated: &'a ParBuf<Key>,
        }

        #[cfg(feature = "parallel")]
        impl<'a, $($type: IntoAbstract),+> $inner_par_update<'a, $($type),+> {
            fn clone(&self) -> Self {
                let iter = $update {
                    data: self.iter.data.clone(),
                    indices: self.iter.indices,
                    current: self.iter.current,
                    end: self.iter.end,
                    array: self.iter.array,
                };

                $inner_par_update {
                    iter,
                    updated: self.updated,
                }
            }
        }

        #[cfg(feature = "parallel")]
        impl<'a, $($type: IntoAbstract),+> UnindexedProducer for $inner_par_update<'a, $($type,)+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn split(mut self) -> (Self, Option<Self>) {
                let len = self.iter.end - self.iter.current;
                if len >= 2 {
                    let mut clone = self.clone();
                    clone.iter.current += len / 2;
                    self.iter.end = clone.iter.current;
                    (self, Some(clone))
                } else {
                    (self, None)
                }
            }
            fn fold_with<Fold>(self, folder: Fold) -> Fold
            where Fold: Folder<Self::Item> {
                let iter: $par_seq_update<$($type),+> = $par_seq_update {
                    updated: self.updated,
                    data: self.iter.data,
                    indices: self.iter.indices,
                    current: self.iter.current,
                    end: self.iter.end,
                    array: self.iter.array,
                };
                folder.consume_iter(iter)
            }
        }

        #[cfg(feature = "parallel")]
        pub struct $par_seq_update<'a, $($type: IntoAbstract),+> {
            data: ($($type::AbsView,)+),
            current: usize,
            end: usize,
            indices: *const Key,
            updated: &'a ParBuf<Key>,
            array: usize,
        }

        #[cfg(feature = "parallel")]
        impl<'a, $($type: IntoAbstract),+> Iterator for $par_seq_update<'a, $($type),+> {
            type Item = ($(<$type::AbsView as AbstractMut>::Out,)+);
            fn next(&mut self) -> Option<Self::Item> {
                while self.current < self.end {
                    let index = unsafe { std::ptr::read(self.indices.add(self.current)) };
                    self.current += 1;
                    let data_indices = ($(
                        if $index == self.array {
                            self.current - 1
                        } else {
                            if let Some(index) = self.data.$index.index_of(index) {
                                index
                            } else {
                                continue
                            }
                        },
                    )+);

                    self.updated.push(index);

                    unsafe {
                        return Some(($(self.data.$index.get_data(data_indices.$index),)+))
                    }
                }
                None
            }
        }
    }
}

macro_rules! iterators {
    (
        $($number: literal)*; $number1: literal $($queue_number: literal)+;
        $($iter: ident)*; $iter1: ident $($queue_iter: ident)+;
        $($par_iter: ident)*; $par_iter1: ident $($queue_par_iter: ident)+;
        $($tight: ident)*; $tight1: ident $($queue_tight: ident)+;
        $($loose: ident)*; $loose1: ident $($queue_loose: ident)+;
        $($non_packed: ident)*; $non_packed1: ident $($queue_non_packed: ident)+;
        $($chunk: ident)*; $chunk1: ident $($queue_chunk: ident)+;
        $($chunk_exact: ident)*; $chunk_exact1: ident $($queue_chunk_exact: ident)+;
        $($par_tight: ident)*; $par_tight1: ident $($queue_par_tight: ident)+;
        $($par_loose: ident)*; $par_loose1: ident $($queue_par_loose: ident)+;
        $($par_non_packed: ident)*; $par_non_packed1: ident $($queue_par_non_packed: ident)+;
        $($filter: ident)*; $filter1: ident $($queue_filter: ident)+;
        $($update: ident)*; $update1: ident $($queue_update: ident)+;
        $($par_update: ident)*; $par_update1: ident $($queue_par_update: ident)+;
        $($inner_par_update: ident)*; $inner_par_update1: ident $($queue_inner_par_update: ident)+;
        $($par_seq_update: ident)*; $par_seq_update1: ident $($queue_par_seq_update: ident)+;
        $(($type: ident, $index: tt))*;($type1: ident, $index1: tt) $(($queue_type: ident, $queue_index: tt))*
    ) => {
        impl_iterators![$number1 $iter1 $par_iter1 $tight1 $loose1 $non_packed1 $chunk1 $chunk_exact1 $par_tight1 $par_loose1 $par_non_packed1 $filter1 $update1 $par_update1 $inner_par_update1 $par_seq_update1 $(($type, $index))*];
        iterators![
            $($number)* $number1; $($queue_number)+;
            $($iter)* $iter1; $($queue_iter)+;
            $($par_iter)* $par_iter1; $($queue_par_iter)+;
            $($tight)* $tight1; $($queue_tight)+;
            $($loose)* $loose1; $($queue_loose)+;
            $($non_packed)* $non_packed1; $($queue_non_packed)+;
            $($chunk)* $chunk1; $($queue_chunk)+;
            $($chunk_exact)* $chunk_exact1; $($queue_chunk_exact)+;
            $($par_tight)* $par_tight1; $($queue_par_tight)+;
            $($par_loose)* $par_loose1; $($queue_par_loose)+;
            $($par_non_packed)* $par_non_packed1; $($queue_par_non_packed)+;
            $($filter)* $filter1; $($queue_filter)+;
            $($update)* $update1; $($queue_update)+;
            $($par_update)* $par_update1; $($queue_par_update)+;
            $($inner_par_update)* $inner_par_update1; $($queue_inner_par_update)+;
            $($par_seq_update)* $par_seq_update1; $($queue_par_seq_update)+;
            $(($type, $index))* ($type1, $index1); $(($queue_type, $queue_index))*
        ];
    };
    (
        $($number: literal)*; $number1: literal;
        $($iter: ident)*; $iter1: ident;
        $($par_iter: ident)*; $par_iter1: ident;
        $($tight: ident)*; $tight1: ident;
        $($loose: ident)*; $loose1: ident;
        $($non_packed: ident)*; $non_packed1: ident;
        $($chunk: ident)*; $chunk1: ident;
        $($chunk_exact: ident)*; $chunk_exact1: ident;
        $($par_tight: ident)*; $par_tight1: ident;
        $($par_loose: ident)*; $par_loose1: ident;
        $($par_non_packed: ident)*; $par_non_packed1: ident;
        $($filter: ident)*; $filter1: ident;
        $($update: ident)*; $update1: ident;
        $($par_update: ident)*; $par_update1: ident;
        $($inner_par_update: ident)*; $inner_par_update1: ident;
        $($par_seq_update: ident)*; $par_seq_update1: ident;
        $(($type: ident, $index: tt))*;
    ) => {
        impl_iterators![$number1 $iter1 $par_iter1 $tight1 $loose1 $non_packed1 $chunk1 $chunk_exact1 $par_tight1 $par_loose1 $par_non_packed1 $filter1 $update1 $par_update1 $inner_par_update1 $par_seq_update1 $(($type, $index))*];
    }
}

iterators![
    ;"2" "3" "4" "5" "6" "7" "8" "9" "10";
    ;Iter2 Iter3 Iter4 Iter5 Iter6 Iter7 Iter8 Iter9 Iter10;
    ;ParIter2 ParIter3 ParIter4 ParIter5 ParIter6 ParIter7 ParIter8 ParIter9 ParIter10;
    ;Tight2 Tight3 Tight4 Tight5 Tight6 Tight7 Tight8 Tight9 Tight10;
    ;Loose2 Loose3 Loose4 Loose5 Loose6 Loose7 Loose8 Loose9 Loose10;
    ;NonPacked2 NonPacked3 NonPacked4 NonPacked5 NonPacked6 NonPacked7 NonPacked8 NonPacked9 NonPacked10;
    ;Chunk2 Chunk3 Chunk4 Chunk5 Chunk6 Chunk7 Chunk8 Chunk9 Chunk10;
    ;ChunkExact2 ChunkExact3 ChunkExact4 ChunkExact5 ChunkExact6 ChunkExact7 ChunkExact8 ChunkExact9 ChunkExact10;
    ;ParTight2 ParTight3 ParTight4 ParTight5 ParTight6 ParTight7 ParTight8 ParTight9 ParTight10;
    ;ParLoose2 ParLoose3 ParLoose4 ParLoose5 ParLoose6 ParLoose7 ParLoose8 ParLoose9 ParLoose10;
    ;ParNonPacked2 ParNonPacked3 ParNonPacked4 ParNonPacked5 ParNonPacked6 ParNonPacked7 ParNonPacked8 ParNonPacked9 ParNonPacked10;
    ;Filter2 Filter3 Filter4 Filter5 Filter6 Filter7 Filter8 Filter9 Filter10;
    ;Update2 Update3 Update4 Update5 Update6 Update7 Update8 Update9 Update10;
    ;ParUpdate2 ParUpdate3 ParUpdate4 ParUpdate5 ParUpdate6 ParUpdate7 ParUpdate8 ParUpdate9 ParUpdate10;
    ;InnerParUpdate2 InnerParUpdate3 InnerParUpdate4 InnerParUpdate5 InnerParUpdate6 InnerParUpdate7 InnerParUpdate8 InnerParUpdate9 InnerParUpdate10;
    ;ParSeqUpdate2 ParSeqUpdate3 ParSeqUpdate4 ParSeqUpdate5 ParSeqUpdate6 ParSeqUpdate7 ParSeqUpdate8 ParSeqUpdate9 ParSeqUpdate10;
    (A, 0) (B, 1); (C, 2) (D, 3) (E, 4) (F, 5) (G, 6) (H, 7) (I, 8) (J, 9)
];
