use std::cell::UnsafeCell;

/// Slot is a more flexible Option.
/// It means you can extend a lot "Slots" with different features with Option type
/// It avoids to modify the pool records when you wanna add new feature like stats
pub trait Slot: Sized {
    type Element;

    fn new_empty() -> Self;

    fn new(element: Self::Element) -> Self;

    fn is_some(&self) -> bool;

    fn as_ref(&self) -> Option<&Self::Element>;

    fn as_mut(&mut self) -> Option<&mut Self::Element>;

    fn replace(&mut self, element: Self::Element) -> Option<Self::Element>;

    fn take(&mut self) -> Option<Self::Element>;
}

/// Impl slot for option make all options be slots
impl<ElementType> Slot for Option<ElementType> {
    type Element = ElementType;

    #[inline]
    fn new_empty() -> Self {
        Self::None
    }

    #[inline]
    fn new(element: Self::Element) -> Self {
        Self::Some(element)
    }

    #[inline]
    fn is_some(&self) -> bool {
        Option::is_some(self)
    }

    #[inline]
    fn as_ref(&self) -> Option<&Self::Element> {
        Option::as_ref(self)
    }

    #[inline]
    fn as_mut(&mut self) -> Option<&mut Self::Element> {
        Option::as_mut(self)
    }

    #[inline]
    fn replace(&mut self, element: Self::Element) -> Option<Self::Element> {
        Option::replace(self, element)
    }

    #[inline]
    fn take(&mut self) -> Option<Self::Element> {
        Option::take(self)
    }
}

/// SlotWrapper warped the slot to have internal mutation
#[derive(Debug)]
pub struct SlotWrapper<S>(pub UnsafeCell<S>);

impl<T, S> Clone for SlotWrapper<S>
where
    T: Sized,
    S: Slot<Element = T> + Clone,
{
    fn clone(&self) -> Self {
        Self(UnsafeCell::new((self.get()).clone()))
    }
}

impl<T, S> SlotWrapper<S>
where
    T: Sized,
    S: Slot<Element = T>,
{
    pub fn new(data: T) -> Self {
        Self(UnsafeCell::new(S::new(data)))
    }

    pub fn new_empty() -> Self {
        Self(UnsafeCell::new(S::new_empty()))
    }

    pub fn get(&self) -> &S {
        unsafe { &*self.0.get() }
    }

    pub fn get_mut(&mut self) -> &mut S {
        self.0.get_mut()
    }

    pub fn is_some(&self) -> bool {
        self.get().is_some()
    }

    #[inline]
    pub fn as_ref(&self) -> Option<&T> {
        self.get().as_ref()
    }

    #[inline]
    pub fn as_mut(&mut self) -> Option<&mut T> {
        self.get_mut().as_mut()
    }

    pub fn replace(&mut self, element: T) -> Option<T> {
        self.get_mut().replace(element)
    }

    pub fn take(&mut self) -> Option<T> {
        self.get_mut().take()
    }
}

unsafe impl<S: Sync> Sync for SlotWrapper<S> {}

unsafe impl<S: Send> Send for SlotWrapper<S> {}
