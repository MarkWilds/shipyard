use crate::all_storages::AllStorages;
use crate::atomic_refcell::{ExclusiveBorrow, Ref, RefMut, SharedBorrow};
use crate::component::Component;
use crate::entities::Entities;
use crate::entity_id::EntityId;
use crate::error;
use crate::get::Get;
use crate::sparse_set::SparseSet;
use crate::storage::StorageId;
use crate::track::{self, Tracking};
use crate::tracking::{Inserted, InsertedOrModified, Modified};
use crate::unique::{TrackingState, Unique};
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

/// Shared view over `AllStorages`.
pub struct AllStoragesView<'a>(pub(crate) Ref<'a, &'a AllStorages>);

impl Clone for AllStoragesView<'_> {
    #[inline]
    fn clone(&self) -> Self {
        AllStoragesView(self.0.clone())
    }
}

impl Deref for AllStoragesView<'_> {
    type Target = AllStorages;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<AllStorages> for AllStoragesView<'_> {
    #[inline]
    fn as_ref(&self) -> &AllStorages {
        &self.0
    }
}

/// Exclusive view over `AllStorages`.
pub struct AllStoragesViewMut<'a>(pub(crate) RefMut<'a, &'a mut AllStorages>);

impl Deref for AllStoragesViewMut<'_> {
    type Target = AllStorages;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AllStoragesViewMut<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<AllStorages> for AllStoragesViewMut<'_> {
    #[inline]
    fn as_ref(&self) -> &AllStorages {
        &self.0
    }
}

impl AsMut<AllStorages> for AllStoragesViewMut<'_> {
    #[inline]
    fn as_mut(&mut self) -> &mut AllStorages {
        &mut self.0
    }
}

/// Shared view over `Entities` storage.
pub struct EntitiesView<'a> {
    pub(crate) entities: &'a Entities,
    pub(crate) borrow: Option<SharedBorrow<'a>>,
    pub(crate) all_borrow: Option<SharedBorrow<'a>>,
}

impl Deref for EntitiesView<'_> {
    type Target = Entities;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.entities
    }
}

impl Clone for EntitiesView<'_> {
    #[inline]
    fn clone(&self) -> Self {
        EntitiesView {
            entities: self.entities,
            borrow: self.borrow.clone(),
            all_borrow: self.all_borrow.clone(),
        }
    }
}

/// Exclusive view over `Entities` storage.
pub struct EntitiesViewMut<'a> {
    pub(crate) entities: &'a mut Entities,
    pub(crate) _borrow: Option<ExclusiveBorrow<'a>>,
    pub(crate) _all_borrow: Option<SharedBorrow<'a>>,
}

impl Deref for EntitiesViewMut<'_> {
    type Target = Entities;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.entities
    }
}

impl DerefMut for EntitiesViewMut<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.entities
    }
}

/// Shared view over a component storage.
pub struct View<'a, T: Component, Tracking: track::Tracking<T> = <T as Component>::Tracking> {
    pub(crate) sparse_set: &'a SparseSet<T, Tracking>,
    pub(crate) all_borrow: Option<SharedBorrow<'a>>,
    pub(crate) borrow: Option<SharedBorrow<'a>>,
    pub(crate) last_insert: u32,
    pub(crate) last_modification: u32,
    pub(crate) current: u32,
}

impl<'a, T: Component> View<'a, T> {
    /// Returns `true` if `entity`'s component was inserted since the last [`clear_all_inserted`] call.  
    /// Returns `false` if `entity` does not have a component in this storage.
    ///
    /// [`clear_all_inserted`]: Self::clear_all_inserted
    #[inline]
    pub fn is_inserted(&self, entity: EntityId) -> bool {
        T::Tracking::is_inserted(self.sparse_set, entity, self.last_insert, self.current)
    }
    /// Returns `true` if `entity`'s component was modified since the last [`clear_all_modified`] call.  
    /// Returns `false` if `entity` does not have a component in this storage.
    ///
    /// [`clear_all_modified`]: Self::clear_all_modified
    #[inline]
    pub fn is_modified(&self, entity: EntityId) -> bool {
        T::Tracking::is_modified(
            self.sparse_set,
            entity,
            self.last_modification,
            self.current,
        )
    }
    /// Returns `true` if `entity`'s component was inserted or modified since the last clear call.  
    /// Returns `false` if `entity` does not have a component in this storage.
    #[inline]
    pub fn is_inserted_or_modified(&self, entity: EntityId) -> bool {
        self.is_inserted(entity) || self.is_modified(entity)
    }
}

impl<'a, T: Component<Tracking = track::Untracked>> View<'a, T, track::Untracked> {
    /// Creates a new [`View`] for custom [`SparseSet`] storage.
    ///
    /// ```
    /// use shipyard::{track, Component, SparseSet, StorageId, View, World};
    ///
    /// struct ScriptingComponent(Vec<u8>);
    /// impl Component for ScriptingComponent {
    ///     type Tracking = track::Untracked;
    /// }
    ///
    /// let world = World::new();
    ///
    /// world.add_custom_storage(
    ///     StorageId::Custom(0),
    ///     SparseSet::<ScriptingComponent>::new_custom_storage(),
    /// ).unwrap();
    ///
    /// let all_storages = world.all_storages().unwrap();
    /// let scripting_storage =
    ///     View::<ScriptingComponent>::new_for_custom_storage(StorageId::Custom(0), all_storages)
    ///         .unwrap();
    /// ```
    pub fn new_for_custom_storage(
        storage_id: StorageId,
        all_storages: Ref<'a, &'a AllStorages>,
    ) -> Result<Self, error::CustomStorageView> {
        use crate::all_storages::CustomStorageAccess;

        let (all_storages, all_borrow) = unsafe { Ref::destructure(all_storages) };

        let storage = all_storages.custom_storage_by_id(storage_id)?;
        let (storage, borrow) = unsafe { Ref::destructure(storage) };

        if let Some(sparse_set) = storage.as_any().downcast_ref() {
            Ok(View {
                sparse_set,
                all_borrow: Some(all_borrow),
                borrow: Some(borrow),
                last_insert: 0,
                last_modification: 0,
                current: 0,
            })
        } else {
            Err(error::CustomStorageView::WrongType(storage.name()))
        }
    }
}

impl<T: Component<Tracking = track::Insertion>> View<'_, T, track::Insertion> {
    /// Wraps this view to be able to iterate *inserted* components.
    #[inline]
    pub fn inserted(&self) -> Inserted<&Self> {
        Inserted(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified(&self) -> InsertedOrModified<&Self> {
        InsertedOrModified(self)
    }
}

impl<T: Component<Tracking = track::Modification>> View<'_, T, track::Modification> {
    /// Wraps this view to be able to iterate *modified* components.
    #[inline]
    pub fn modified(&self) -> Modified<&Self> {
        Modified(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified(&self) -> InsertedOrModified<&Self> {
        InsertedOrModified(self)
    }
}

impl<T: Component<Tracking = track::All>> View<'_, T, track::All> {
    /// Wraps this view to be able to iterate *inserted* components.
    #[inline]
    pub fn inserted(&self) -> Inserted<&Self> {
        Inserted(self)
    }
    /// Wraps this view to be able to iterate *modified* components.
    #[inline]
    pub fn modified(&self) -> Modified<&Self> {
        Modified(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified(&self) -> InsertedOrModified<&Self> {
        InsertedOrModified(self)
    }
}

impl<'a, T: Component> Deref for View<'a, T> {
    type Target = SparseSet<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.sparse_set
    }
}

impl<'a, T: Component> AsRef<SparseSet<T>> for View<'a, T> {
    #[inline]
    fn as_ref(&self) -> &SparseSet<T> {
        self.sparse_set
    }
}

impl<'a, T: Component> Clone for View<'a, T> {
    #[inline]
    fn clone(&self) -> Self {
        View {
            sparse_set: self.sparse_set,
            borrow: self.borrow.clone(),
            all_borrow: self.all_borrow.clone(),
            last_insert: self.last_insert,
            last_modification: self.last_modification,
            current: self.current,
        }
    }
}

impl<T: fmt::Debug + Component> fmt::Debug for View<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.sparse_set.fmt(f)
    }
}

impl<T: Component> core::ops::Index<EntityId> for View<'_, T> {
    type Output = T;
    #[track_caller]
    #[inline]
    fn index(&self, entity: EntityId) -> &Self::Output {
        self.get(entity).unwrap()
    }
}

/// Exclusive view over a component storage.
pub struct ViewMut<'a, T: Component, Tracking: track::Tracking<T> = <T as Component>::Tracking> {
    pub(crate) sparse_set: &'a mut SparseSet<T, Tracking>,
    pub(crate) _all_borrow: Option<SharedBorrow<'a>>,
    pub(crate) _borrow: Option<ExclusiveBorrow<'a>>,
    pub(crate) last_insert: u32,
    pub(crate) last_modification: u32,
    pub(crate) current: u32,
}

impl<'a, T: Component<Tracking = track::Untracked>> ViewMut<'a, T, track::Untracked> {
    /// Creates a new [`ViewMut`] for custom [`SparseSet`] storage.
    ///
    /// ```
    /// use shipyard::{track, Component, SparseSet, StorageId, ViewMut, World};
    ///
    /// struct ScriptingComponent(Vec<u8>);
    /// impl Component for ScriptingComponent {
    ///     type Tracking = track::Untracked;
    /// }
    ///
    /// let world = World::new();
    ///
    /// world.add_custom_storage(
    ///     StorageId::Custom(0),
    ///     SparseSet::<ScriptingComponent>::new_custom_storage(),
    /// ).unwrap();
    ///
    /// let all_storages = world.all_storages().unwrap();
    /// let scripting_storage =
    ///     ViewMut::<ScriptingComponent>::new_for_custom_storage(StorageId::Custom(0), all_storages)
    ///         .unwrap();
    /// ```
    pub fn new_for_custom_storage(
        storage_id: StorageId,
        all_storages: Ref<'a, &'a AllStorages>,
    ) -> Result<Self, error::CustomStorageView> {
        use crate::all_storages::CustomStorageAccess;

        let (all_storages, all_borrow) = unsafe { Ref::destructure(all_storages) };

        let storage = all_storages.custom_storage_mut_by_id(storage_id)?;
        let (storage, borrow) = unsafe { RefMut::destructure(storage) };

        let name = storage.name();

        if let Some(sparse_set) = storage.any_mut().downcast_mut() {
            Ok(ViewMut {
                sparse_set,
                _all_borrow: Some(all_borrow),
                _borrow: Some(borrow),
                last_insert: 0,
                last_modification: 0,
                current: 0,
            })
        } else {
            Err(error::CustomStorageView::WrongType(name))
        }
    }
}

impl<'a, T: Component> ViewMut<'a, T> {
    /// Applies the given function `f` to the entities `a` and `b`.  
    /// The two entities shouldn't point to the same component.  
    ///
    /// ### Panics
    ///
    /// - MissingComponent - if one of the entity doesn't have any component in the storage.
    /// - IdenticalIds - if the two entities point to the same component.
    #[track_caller]
    #[inline]
    pub fn apply<R, F: FnOnce(&mut T, &T) -> R>(&mut self, a: EntityId, b: EntityId, f: F) -> R {
        T::Tracking::apply(self, a, b, f)
    }
    /// Applies the given function `f` to the entities `a` and `b`.  
    /// The two entities shouldn't point to the same component.  
    ///
    /// ### Panics
    ///
    /// - MissingComponent - if one of the entity doesn't have any component in the storage.
    /// - IdenticalIds - if the two entities point to the same component.
    #[track_caller]
    #[inline]
    pub fn apply_mut<R, F: FnOnce(&mut T, &mut T) -> R>(
        &mut self,
        a: EntityId,
        b: EntityId,
        f: F,
    ) -> R {
        T::Tracking::apply_mut(self, a, b, f)
    }
    /// Returns `true` if `entity`'s component was inserted since the last [`clear_all_inserted`] call.  
    /// Returns `false` if `entity` does not have a component in this storage.
    ///
    /// [`clear_all_inserted`]: Self::clear_all_inserted
    #[inline]
    pub fn is_inserted(&self, entity: EntityId) -> bool {
        T::Tracking::is_inserted(self.sparse_set, entity, self.last_insert, self.current)
    }
    /// Returns `true` if `entity`'s component was modified since the last [`clear_all_modified`] call.  
    /// Returns `false` if `entity` does not have a component in this storage.
    ///
    /// [`clear_all_modified`]: Self::clear_all_modified
    #[inline]
    pub fn is_modified(&self, entity: EntityId) -> bool {
        T::Tracking::is_modified(
            self.sparse_set,
            entity,
            self.last_modification,
            self.current,
        )
    }
    /// Returns `true` if `entity`'s component was inserted or modified since the last clear call.  
    /// Returns `false` if `entity` does not have a component in this storage.
    #[inline]
    pub fn is_inserted_or_modified(&self, entity: EntityId) -> bool {
        self.is_inserted(entity) || self.is_modified(entity)
    }
}

impl<T: Component<Tracking = track::Insertion>> ViewMut<'_, T, track::Insertion> {
    /// Wraps this view to be able to iterate *inserted* components.
    #[inline]
    pub fn inserted(&self) -> Inserted<&Self> {
        Inserted(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified(&self) -> InsertedOrModified<&Self> {
        InsertedOrModified(self)
    }
    /// Wraps this view to be able to iterate *inserted* components.
    #[inline]
    pub fn inserted_mut(&mut self) -> Inserted<&mut Self> {
        Inserted(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified_mut(&mut self) -> InsertedOrModified<&mut Self> {
        InsertedOrModified(self)
    }
    /// Removes the *inserted* flag on all components of this storage.
    #[inline]
    pub fn clear_all_inserted(self) {
        self.sparse_set.private_clear_all_inserted(self.current);
    }
}

impl<T: Component<Tracking = track::Modification>> ViewMut<'_, T, track::Modification> {
    /// Wraps this view to be able to iterate *modified* components.
    #[inline]
    pub fn modified(&self) -> Modified<&Self> {
        Modified(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified(&self) -> InsertedOrModified<&Self> {
        InsertedOrModified(self)
    }
    /// Wraps this view to be able to iterate *modified* components.
    #[inline]
    pub fn modified_mut(&mut self) -> Modified<&mut Self> {
        Modified(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified_mut(&mut self) -> InsertedOrModified<&mut Self> {
        InsertedOrModified(self)
    }
    /// Removes the *modified* flag on all components of this storage.
    #[inline]
    pub fn clear_all_modified(self) {
        self.sparse_set.private_clear_all_modified(self.current);
    }
}

impl<T: Component<Tracking = track::All>> ViewMut<'_, T, track::All> {
    /// Wraps this view to be able to iterate *inserted* components.
    #[inline]
    pub fn inserted(&self) -> Inserted<&Self> {
        Inserted(self)
    }
    /// Wraps this view to be able to iterate *modified* components.
    #[inline]
    pub fn modified(&self) -> Modified<&Self> {
        Modified(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified(&self) -> InsertedOrModified<&Self> {
        InsertedOrModified(self)
    }
    /// Wraps this view to be able to iterate *inserted* components.
    #[inline]
    pub fn inserted_mut(&mut self) -> Inserted<&mut Self> {
        Inserted(self)
    }
    /// Wraps this view to be able to iterate *modified* components.
    #[inline]
    pub fn modified_mut(&mut self) -> Modified<&mut Self> {
        Modified(self)
    }
    /// Wraps this view to be able to iterate *inserted* and *modified* components.
    #[inline]
    pub fn inserted_or_modified_mut(&mut self) -> InsertedOrModified<&mut Self> {
        InsertedOrModified(self)
    }
    /// Removes the *inserted* flag on all components of this storage.
    #[inline]
    pub fn clear_all_inserted(self) {
        self.sparse_set.private_clear_all_inserted(self.current);
    }
    /// Removes the *modified* flag on all components of this storage.
    #[inline]
    pub fn clear_all_modified(self) {
        self.sparse_set.private_clear_all_modified(self.current);
    }
    /// Removes the *inserted* and *modified* flags on all components of this storage.
    #[inline]
    pub fn clear_all_inserted_and_modified(self) {
        self.sparse_set
            .private_clear_all_inserted_and_modified(self.current);
    }
}

impl<T: Component> Deref for ViewMut<'_, T> {
    type Target = SparseSet<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.sparse_set
    }
}

impl<T: Component> DerefMut for ViewMut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.sparse_set
    }
}

impl<'a, T: Component> AsRef<SparseSet<T>> for ViewMut<'a, T> {
    #[inline]
    fn as_ref(&self) -> &SparseSet<T> {
        self.sparse_set
    }
}

impl<'a, T: Component> AsMut<SparseSet<T>> for ViewMut<'a, T> {
    #[inline]
    fn as_mut(&mut self) -> &mut SparseSet<T> {
        self.sparse_set
    }
}

impl<'a, T: Component> AsMut<Self> for ViewMut<'a, T> {
    #[inline]
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

impl<T: fmt::Debug + Component> fmt::Debug for ViewMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.sparse_set.fmt(f)
    }
}

impl<'a, T: Component> core::ops::Index<EntityId> for ViewMut<'a, T> {
    type Output = T;
    #[inline]
    fn index(&self, entity: EntityId) -> &Self::Output {
        self.get(entity).unwrap()
    }
}

impl<'a, T: Component<Tracking = track::Untracked>> core::ops::IndexMut<EntityId>
    for ViewMut<'a, T, track::Untracked>
{
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        (&mut *self).get(entity).unwrap()
    }
}

impl<'a, T: Component<Tracking = track::Insertion>> core::ops::IndexMut<EntityId>
    for ViewMut<'a, T, track::Insertion>
{
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        (&mut *self).get(entity).unwrap()
    }
}

impl<'a, T: Component<Tracking = track::Deletion>> core::ops::IndexMut<EntityId>
    for ViewMut<'a, T, track::Deletion>
{
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        (&mut *self).get(entity).unwrap()
    }
}

impl<'a, T: Component<Tracking = track::Removal>> core::ops::IndexMut<EntityId>
    for ViewMut<'a, T, track::Removal>
{
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        (&mut *self).get(entity).unwrap()
    }
}

impl<'a, T: Component<Tracking = track::Modification>> core::ops::IndexMut<EntityId>
    for ViewMut<'a, T, track::Modification>
{
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        let index = self
            .index_of(entity)
            .ok_or_else(|| error::MissingComponent {
                id: entity,
                name: core::any::type_name::<T>(),
            })
            .unwrap();

        let SparseSet {
            data,
            modification_data,
            ..
        } = self.sparse_set;

        unsafe {
            *modification_data.get_unchecked_mut(index) = self.current;
        };

        unsafe { data.get_unchecked_mut(index) }
    }
}

impl<'a, T: Component<Tracking = track::All>> core::ops::IndexMut<EntityId>
    for ViewMut<'a, T, track::All>
{
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        let index = self
            .index_of(entity)
            .ok_or_else(|| error::MissingComponent {
                id: entity,
                name: core::any::type_name::<T>(),
            })
            .unwrap();

        let SparseSet {
            data,
            modification_data,
            ..
        } = self.sparse_set;

        unsafe {
            *modification_data.get_unchecked_mut(index) = self.current;
        };

        unsafe { data.get_unchecked_mut(index) }
    }
}

/// Shared view over a unique component storage.
pub struct UniqueView<'a, T: Component, Track: Tracking<T> = <T as Component>::Tracking> {
    pub(crate) unique: &'a Unique<T>,
    pub(crate) borrow: Option<SharedBorrow<'a>>,
    pub(crate) all_borrow: Option<SharedBorrow<'a>>,
    pub(crate) _phantom: PhantomData<Track>,
}

impl<T: Component> UniqueView<'_, T> {
    /// Duplicates the [`UniqueView`].
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn clone(unique: &Self) -> Self {
        UniqueView {
            unique: unique.unique,
            borrow: unique.borrow.clone(),
            all_borrow: unique.all_borrow.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T: Component<Tracking = track::Insertion>> UniqueView<'_, T, track::Insertion> {
    /// Returns `true` if the component was inserted before the last [`clear_inserted`] call.  
    ///
    /// [`clear_inserted`]: UniqueViewMut::clear_inserted
    #[inline]
    pub fn is_inserted(&self) -> bool {
        self.unique.tracking == TrackingState::Inserted
    }
    /// Returns `true` if the component was inserted before the last [`clear_inserted`] call.  
    ///
    /// [`clear_inserted`]: UniqueViewMut::clear_inserted
    #[inline]
    pub fn is_inserted_or_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Inserted
    }
}

impl<T: Component<Tracking = track::Modification>> UniqueView<'_, T, track::Modification> {
    /// Returns `true` is the component was modified since the last [`clear_modified`] call.
    ///
    /// [`clear_modified`]: UniqueViewMut::clear_modified
    #[inline]
    pub fn is_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Modified
    }
    /// Returns `true` if the component was modified since the last [`clear_modified`] call.  
    ///
    /// [`clear_modified`]: UniqueViewMut::clear_modified
    #[inline]
    pub fn is_inserted_or_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Modified
    }
}

impl<T: Component<Tracking = track::All>> UniqueView<'_, T, track::All> {
    /// Returns `true` if the component was inserted before the last [`clear_inserted`] call.  
    ///
    /// [`clear_inserted`]: UniqueViewMut::clear_inserted
    #[inline]
    pub fn is_inserted(&self) -> bool {
        self.unique.tracking == TrackingState::Inserted
    }
    /// Returns `true` is the component was modified since the last [`clear_modified`] call.
    ///
    /// [`clear_modified`]: UniqueViewMut::clear_modified
    #[inline]
    pub fn is_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Modified
    }
    /// Returns `true` if the component was inserted or modified since the last [`clear_inserted`] or [`clear_modified`] call.  
    ///
    /// [`clear_inserted`]: UniqueViewMut::clear_inserted
    /// [`clear_modified`]: UniqueViewMut::clear_modified
    #[inline]
    pub fn is_inserted_or_modified(&self) -> bool {
        self.unique.tracking != TrackingState::Untracked
    }
}

impl<T: Component> Deref for UniqueView<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.unique.value
    }
}

impl<T: Component> AsRef<T> for UniqueView<'_, T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.unique.value
    }
}

impl<T: fmt::Debug + Component> fmt::Debug for UniqueView<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.unique.value.fmt(f)
    }
}

/// Exclusive view over a unique component storage.
pub struct UniqueViewMut<'a, T: Component, Track: Tracking<T> = <T as Component>::Tracking> {
    pub(crate) unique: &'a mut Unique<T>,
    pub(crate) _borrow: Option<ExclusiveBorrow<'a>>,
    pub(crate) _all_borrow: Option<SharedBorrow<'a>>,
    pub(crate) _phantom: PhantomData<Track>,
}

impl<T: Component<Tracking = track::Insertion>> UniqueViewMut<'_, T, track::Insertion> {
    /// Returns `true` if the component was inserted before the last [`clear_inserted`] call.  
    ///
    /// [`clear_inserted`]: Self::clear_inserted
    #[inline]
    pub fn is_inserted(&self) -> bool {
        self.unique.tracking == TrackingState::Inserted
    }
    /// Returns `true` if the component was inserted before the last [`clear_inserted`] call.  
    ///
    /// [`clear_inserted`]: Self::clear_inserted
    #[inline]
    pub fn is_inserted_or_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Inserted
    }
    /// Removes the *inserted* flag on the component of this storage.
    #[inline]
    pub fn clear_inserted(&mut self) {
        self.unique.tracking = TrackingState::Untracked;
    }
}

impl<T: Component<Tracking = track::Modification>> UniqueViewMut<'_, T, track::Modification> {
    /// Returns `true` if the component was modified since the last [`clear_modified`] call.  
    ///
    /// [`clear_modified`]: Self::clear_modified
    #[inline]
    pub fn is_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Modified
    }
    /// Returns `true` if the component was modified since the last [`clear_modified`] call.  
    ///
    /// [`clear_modified`]: Self::clear_modified
    #[inline]
    pub fn is_inserted_or_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Modified
    }
    /// Removes the *medified* flag on the component of this storage.
    #[inline]
    pub fn clear_modified(&mut self) {
        self.unique.tracking = TrackingState::Untracked;
    }
}

impl<T: Component<Tracking = track::All>> UniqueViewMut<'_, T, track::All> {
    /// Returns `true` if the component was inserted before the last [`clear_inserted`] call.  
    ///
    /// [`clear_inserted`]: Self::clear_inserted
    #[inline]
    pub fn is_inserted(&self) -> bool {
        self.unique.tracking == TrackingState::Inserted
    }
    /// Returns `true` if the component was modified since the last [`clear_modified`] call.  
    ///
    /// [`clear_modified`]: Self::clear_modified
    #[inline]
    pub fn is_modified(&self) -> bool {
        self.unique.tracking == TrackingState::Modified
    }
    /// Returns `true` if the component was inserted or modified since the last [`clear_inserted`] or [`clear_modified`] call.  
    ///
    /// [`clear_inserted`]: Self::clear_inserted
    /// [`clear_modified`]: Self::clear_modified
    #[inline]
    pub fn is_inserted_or_modified(&self) -> bool {
        self.unique.tracking != TrackingState::Untracked
    }
    /// Removes the *inserted* flag on the component of this storage.
    #[inline]
    pub fn clear_inserted(&mut self) {
        if self.unique.tracking == TrackingState::Inserted {
            self.unique.tracking = TrackingState::Untracked;
        }
    }
    /// Removes the *medified* flag on the component of this storage.
    #[inline]
    pub fn clear_modified(&mut self) {
        if self.unique.tracking == TrackingState::Modified {
            self.unique.tracking = TrackingState::Untracked;
        }
    }
    /// Removes the *inserted* and *modified* flags on the component of this storage.
    #[inline]
    pub fn clear_inserted_and_modified(&mut self) {
        self.unique.tracking = TrackingState::Untracked;
    }
}

impl<T: Component> Deref for UniqueViewMut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.unique.value
    }
}

impl<T: Component<Tracking = track::Untracked>> DerefMut
    for UniqueViewMut<'_, T, track::Untracked>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::Insertion>> DerefMut
    for UniqueViewMut<'_, T, track::Insertion>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::Removal>> DerefMut for UniqueViewMut<'_, T, track::Removal> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::Modification>> DerefMut
    for UniqueViewMut<'_, T, track::Modification>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.unique.tracking = TrackingState::Modified;

        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::All>> DerefMut for UniqueViewMut<'_, T, track::All> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        if self.unique.tracking == TrackingState::Untracked {
            self.unique.tracking = TrackingState::Modified;
        }

        &mut self.unique.value
    }
}

impl<T: Component> AsRef<T> for UniqueViewMut<'_, T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.unique.value
    }
}

impl<T: Component<Tracking = track::Untracked>> AsMut<T>
    for UniqueViewMut<'_, T, track::Untracked>
{
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::Insertion>> AsMut<T>
    for UniqueViewMut<'_, T, track::Insertion>
{
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::Removal>> AsMut<T> for UniqueViewMut<'_, T, track::Removal> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::Modification>> AsMut<T>
    for UniqueViewMut<'_, T, track::Modification>
{
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self.unique.tracking = TrackingState::Modified;

        &mut self.unique.value
    }
}

impl<T: Component<Tracking = track::All>> AsMut<T> for UniqueViewMut<'_, T, track::All> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        if self.unique.tracking == TrackingState::Untracked {
            self.unique.tracking = TrackingState::Modified;
        }

        &mut self.unique.value
    }
}

impl<T: fmt::Debug + Component> fmt::Debug for UniqueViewMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.unique.value.fmt(f)
    }
}
