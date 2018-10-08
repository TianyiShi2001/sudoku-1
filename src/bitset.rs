use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not, BitXor, BitXorAssign};
use helper::Unsolvable;
use board::{Digit, Cell, Line, House, Position};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Set<T: SetElement>(pub(crate) T::Storage);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Iter<T: SetElement>(T::Storage);

impl<T: SetElement> IntoIterator for Set<T>
where
    Iter<T>: Iterator,
{
    type Item = <Iter<T> as Iterator>::Item;
    type IntoIter = Iter<T>;

    fn into_iter(self) -> Self::IntoIter {
        Iter(self.0)
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////
//                                  Bitops
///////////////////////////////////////////////////////////////////////////////////////////////

macro_rules! impl_binary_bitops {
    ( $( $trait:ident, $fn_name:ident);* $(;)* ) => {
        $(
            impl<T: SetElement> $trait for Set<T> {
                type Output = Self;

                #[inline(always)]
                fn $fn_name(self, other: Self) -> Self {
                    Set(
                        $trait::$fn_name(self.0, other.0)
                    )
                }
            }

            impl<T: SetElement> $trait<T> for Set<T> {
                type Output = Self;

                #[inline(always)]
                fn $fn_name(self, other: T) -> Self {
                    $trait::$fn_name(self, other.as_set())
                }
            }
        )*
    };
}

macro_rules! impl_bitops_assign {
    ( $( $trait:ident, $fn_name:ident);* $(;)* ) => {
        $(
            impl<T: SetElement> $trait for Set<T> {
                #[inline(always)]
                fn $fn_name(&mut self, other: Self) {
                    $trait::$fn_name(&mut self.0, other.0)
                }
            }

            impl<T: SetElement> $trait<T> for Set<T> {
                #[inline(always)]
                fn $fn_name(&mut self, other: T) {
                    $trait::$fn_name(self, other.as_set())
                }
            }
        )*
    };
}

impl_binary_bitops!(
    BitAnd, bitand;
    BitOr, bitor;
    BitXor, bitxor;
);

impl_bitops_assign!(
    BitAndAssign, bitand_assign;
    BitOrAssign, bitor_assign;
    BitXorAssign, bitxor_assign;
);

impl<T: SetElement> Not for Set<T>
where
    Self: PartialEq + Copy
{
    type Output = Self;
    fn not(self) -> Self {
        Self::ALL.without(self)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Zero;

impl From<Zero> for Unsolvable {
    fn from(_: Zero) -> Unsolvable {
        Unsolvable
    }
}

impl<T: SetElement> Set<T>
where
    // TODO: properly implement the traits for Set and Iter
    //       bounded on T::Storage, not on T (which derive does)
    Self: PartialEq + Copy
{
    pub const ALL: Set<T> = Set(<T as SetElement>::ALL);
    pub const NONE: Set<T> = Set(<T as SetElement>::NONE);

    pub fn new(mask: T::Storage) -> Self {
        Set(mask)
    }

    pub fn without(self, other: Self) -> Self {
        Set(self.0 & !other.0)
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    pub fn overlaps(&self, other: Self) -> bool {
        *self & other != Set::NONE
    }

    pub fn len(&self) -> u8 {
        T::count_possibilities(self.0) as u8
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        *self == Self::ALL
    }

    pub fn unique(self) -> Result<Option<T>, Zero>
    where
        Iter<T>: Iterator<Item = T>,
    {
        match self.len() {
            1 => {
                let element = self.into_iter().next();
                debug_assert!(element.is_some());
                Ok(element)
            }
            0 => Err(Zero),
            _ => Ok(None),
        }
    }

    pub fn one_possibility(self) -> T
    where
        Iter<T>: Iterator<Item = T>,
    {
        self.into_iter().next().expect("mask is empty")
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////

use self::set_element::SetElement;
mod set_element {
    use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not, BitXor, BitXorAssign};
    use super::Set;

    pub trait SetElement: Sized {
        const ALL: Self::Storage;
        const NONE: Self::Storage;

        type Storage:
            BitAnd<Output = Self::Storage> + BitAndAssign
            + BitOr<Output = Self::Storage> + BitOrAssign
            + BitXor<Output = Self::Storage> + BitXorAssign
            + Not<Output = Self::Storage>
            + Copy;

        fn count_possibilities(set: Self::Storage) -> u32;
        fn as_set(self) -> Set<Self>;
    }
}

macro_rules! impl_setelement {
    ( $( $type:ty => $storage_ty:ty, $all:expr),* $(,)* ) => {
        $(
            impl SetElement for $type {
                const ALL: $storage_ty = $all;
                const NONE: $storage_ty = 0;

                type Storage = $storage_ty;

                fn count_possibilities(set: Self::Storage) -> u32 {
                    set.count_ones()
                }

                fn as_set(self) -> Set<Self> {
                    Set(1 << self.as_index() as u8)
                }
            }

            impl $type {
                pub fn as_set(self) -> Set<Self> {
                    SetElement::as_set(self)
                }
            }
        )*
    };
}

impl_setelement!(
    // 81 cells
    Cell => u128, 0o777_777_777___777_777_777___777_777_777,
    // 9 digits
    Digit => u16, 0o777,

    // 9 of each house
    //Row => u16, 0o777,
    //Col => u16, 0o777,
    //Block => u16, 0o777,
    Line => u32, 0o777_777,      // both Rows and Cols
    //House => u32, 0o777_777_777, // Rows, Cols, Blocks

    // 9 positions per house
    //Position<Row> => u16, 0o777,
    //Position<Col> => u16, 0o777,
    Position<Line> => u16, 0o777,
    Position<House> => u16, 0o777,
    // 27 positions per chute
    //Position<Band> => u32, 0o777_777_777,
    //Position<Stack> => u32, 0o777_777_777,
    //Position<Chute> => u32, 0o777_777_777,
);

macro_rules! impl_iter_for_setiter {
    ( $( $type:ty => $constructor:expr ),* $(,)* ) => {
        $(
            impl Iterator for Iter<$type> {
                type Item = $type;

                fn next(&mut self) -> Option<Self::Item> {
                    debug_assert!(self.0 <= <Set<$type>>::ALL.0, "{:o}", self.0);
                    if self.0 == 0 {
                        return None;
                    }
                    let lowest_bit = self.0 & (!self.0 + 1);
                    let bit_pos = lowest_bit.trailing_zeros() as u8;
                    self.0 ^= lowest_bit;
                    Some($constructor(bit_pos))
                }
            }
        )*
    };
}

// can't do this generically
impl_iter_for_setiter!(
    Cell => Cell::new,
    Digit => Digit::from_index,
    Line => Line::new,
    //Position<Row> => Position::new,
    //Position<Col> => Position::new,
    Position<Line> => Position::new,
    Position<House> => Position::new,
    //Position<Band> => Position::new,
    //Position<Stack> => Position::new,
    //Position<Chute> => Position::new,
);