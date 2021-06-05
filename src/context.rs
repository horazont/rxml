use std::fmt;
use std::borrow::Cow;

#[cfg(all(feature = "shared_ns", feature = "mt"))]
use std::sync::{Weak, Mutex, MutexGuard};
#[cfg(all(feature = "shared_ns", not(feature = "mt")))]
use std::rc::Weak;
#[cfg(all(feature = "shared_ns", not(feature = "mt")))]
use std::cell::{RefCell, RefMut};

use crate::strings;
use crate::parser::RcPtr;

#[cfg(feature = "shared_ns")]
use weak_table;

#[cfg(feature = "shared_ns")]
type CDataWeakSet = weak_table::WeakHashSet<Weak<strings::CData>>;

/**
# Shared context for multiple parsers

This context allows parsers to share data. This is useful in cases where many
parsers are used in the same application, and all of them encountering similar
data.

As of writing, the context is only used to share namespace URIs encountered in
XML documents, and only if the `shared_ns` feature is used for building.

Even though the context is internally mutable, it can safely be shared with
an immutable reference between parsers. If the crate is built with the `mt`
feature, the Context is Send and Sync, otherwise it is neither.
*/
pub struct Context {
	#[cfg(all(feature = "shared_ns", feature = "mt"))]
	nss: Mutex<CDataWeakSet>,
	#[cfg(all(feature = "shared_ns", not(feature = "mt")))]
	nss: RefCell<CDataWeakSet>,
}

impl Context {
	#[cfg(all(feature = "shared_ns", feature = "mt"))]
	fn wrap_nss(nss: CDataWeakSet) -> Mutex<CDataWeakSet> {
		return Mutex::new(nss)
	}

	#[cfg(all(feature = "shared_ns", not(feature = "mt")))]
	fn wrap_nss(nss: CDataWeakSet) -> RefCell<CDataWeakSet> {
		return RefCell::new(nss)
	}

	/// Create a new context
	pub fn new() -> Context {
		Context{
			#[cfg(feature = "shared_ns")]
			nss: Self::wrap_nss(weak_table::WeakHashSet::new()),
		}
	}

	#[cfg(all(feature = "shared_ns", feature = "mt"))]
	fn lock_nss<'a>(&'a self) -> MutexGuard<'a, CDataWeakSet> {
		self.nss.lock().unwrap()
	}

	#[cfg(all(feature = "shared_ns", not(feature = "mt")))]
	fn lock_nss<'a>(&'a self) -> RefMut<'a, CDataWeakSet> {
		self.nss.borrow_mut()
	}

	/// Intern a piece of text
	///
	/// The given cdata is interned in the context and a refcounted pointer
	/// is returned. When the last reference to that pointer expires, the
	/// string will be lazily removed from the internal storage.
	///
	/// The optimal course is taken depending on whether the Cow is borrowed
	/// or owned.
	///
	/// To force expiry, call [`Context::release_temporaries`], although that
	/// should only rarely be necessary and may be detrimental to performance.
	pub fn intern_cdata<'a, T: Into<Cow<'a, strings::CDataStr>>>(&self, ns: T) -> RcPtr<strings::CData> {
		let ns = ns.into();
		#[cfg(feature = "shared_ns")]
		{
			let mut nss = self.lock_nss();
			return match nss.get(&*ns) {
				Some(ptr) => ptr.clone(),
				None => {
					let ptr = RcPtr::new(ns.into_owned());
					nss.insert(ptr.clone());
					ptr
				},
			}
		}
		#[cfg(not(feature = "shared_ns"))]
		return RcPtr::new(ns.into_owned())
	}

	/// Remove all unreferenced strings from storage and shrink the storage to
	/// fit the requirements.
	///
	/// This should rarely be necessary to call. The internal storage will
	/// prefer expiring unused strings over reallocating and will only
	/// reallocate if necessary.
	pub fn release_temporaries(&self) {
		#[cfg(feature = "shared_ns")]
		{
			let mut nss = self.lock_nss();
			nss.remove_expired();
			nss.shrink_to_fit();
		}
	}

	/// Return the number of CData strings interned.
	///
	/// Returns zero if built without `shared_ns`. This count includes strings
	/// which are unreferenced and which would be removed before the next
	/// reallocation.
	pub fn cdatas(&self) -> usize {
		#[cfg(feature = "shared_ns")]
		{
			let mut nss = self.lock_nss();
			nss.len()
		}
		#[cfg(not(feature = "shared_ns"))]
		0
	}
}

impl fmt::Debug for Context {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		let mut f = f.debug_struct("Context");
		f.field("instance", &(self as *const Context));
		#[cfg(feature = "shared_ns")]
		{
			let nss = self.lock_nss();
			f.field("nss.capacity()", &nss.capacity()).field("nss.length()", &nss.len());
		}
		f.finish()
	}
}
