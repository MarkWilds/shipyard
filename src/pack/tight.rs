use crate::error;
use crate::sparse_set::{Pack, TightPack as TightPackInfo};
use crate::views::ViewMut;
use std::any::TypeId;
use std::sync::Arc;

pub trait TightPack {
    fn try_tight_pack(self) -> Result<(), error::Pack>;
    fn tight_pack(self);
}

macro_rules! impl_tight_pack {
    ($(($type: ident, $index: tt))+) => {
        #[allow(clippy::useless_let_if_seq)]
        impl<$($type: 'static),+> TightPack for ($(&mut ViewMut<'_, $type>,)+) {
            fn try_tight_pack(self) -> Result<(), error::Pack> {
                let mut type_ids: Box<[_]> = Box::new([$(TypeId::of::<$type>(),)+]);

                type_ids.sort_unstable();
                let type_ids: Arc<[_]> = type_ids.into();

                $(
                    match self.$index.pack_info.pack {
                        Pack::Tight(_) => {
                            return Err(error::Pack::AlreadyTightPack(TypeId::of::<$type>()));
                        },
                        Pack::Loose(_) => {
                            return Err(error::Pack::AlreadyLoosePack(TypeId::of::<$type>()));
                        },
                        Pack::Update(_) => {
                            return Err(error::Pack::AlreadyUpdatePack(TypeId::of::<$type>()))
                        },
                        Pack::NoPack => {
                            self.$index.pack_info.pack = Pack::Tight(TightPackInfo::new(Arc::clone(&type_ids)));
                        }
                    }
                )+

                let mut smallest = std::usize::MAX;
                let mut smallest_index = 0;
                let mut i = 0;

                $(
                    if self.$index.len() < smallest {
                        smallest = self.$index.len();
                        smallest_index = i;
                    }
                    i += 1;
                )+
                let _ = smallest;
                let _ = i;

                let mut indices = vec![];

                $(
                    if $index == smallest_index {
                        indices = self.$index.clone_indices();
                    }
                )+

                for index in indices {
                    $(
                        if !self.$index.contains(index) {
                            continue
                        }
                    )+
                    $(
                        self.$index.pack(index);
                    )+
                }

                Ok(())
            }
            fn tight_pack(self) {
                self.try_tight_pack().unwrap()
            }
        }
    }
}

macro_rules! tight_pack {
    ($(($type: ident, $index: tt))*;($type1: ident, $index1: tt) $(($queue_type: ident, $queue_index: tt))*) => {
        impl_tight_pack![$(($type, $index))*];
        tight_pack![$(($type, $index))* ($type1, $index1); $(($queue_type, $queue_index))*];
    };
    ($(($type: ident, $index: tt))*;) => {
        impl_tight_pack![$(($type, $index))*];
    }
}

tight_pack![(A, 0) (B, 1); (C, 2) (D, 3) (E, 4) (F, 5) (G, 6) (H, 7) (I, 8) (J, 9)];