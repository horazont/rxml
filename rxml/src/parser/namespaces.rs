use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
#[cfg(not(feature = "mt"))]
use std::rc::Rc;
#[cfg(feature = "mt")]
use std::sync::Arc;

use crate::context;
use crate::errctx;
use crate::error::{add_context, Error, NWFError, Result, WFError};
use crate::strings::*;

use super::common::{EventMetrics, XMLVersion, XMLNS_XML};
use super::raw::{RawEvent, RawQName};

/// Shared namespace URI
pub type NamespaceName = RcPtr<CData>;

/// Pair of an optional namespace name (URI) and a localpart, commonly used in
/// element and attribute names.
pub type ResolvedQName = (Option<NamespaceName>, NCName);

/// Wrapper pointer around namespace URIs
///
/// In builds with the `mt` feature, this is a [`Arc`]. In non-`mt` builds,
/// this is a [`std::rc::Rc`]
#[cfg(feature = "mt")]
pub type RcPtr<T> = Arc<T>;
/// Wrapper pointer around namespace URIs
///
/// In builds with the `mt` feature, this is a [`std::sync::Arc`].
/// In non-`mt` builds, this is a [`Rc`].
#[cfg(not(feature = "mt"))]
pub type RcPtr<T> = Rc<T>;

/**
# High-level, logical XML document parts

The term *Event* is borrowed from SAX terminology. Each [`ResolvedEvent`]
refers to a bit of the XML document which has been parsed.

In contrast to the [`RawEvent`], observing a [`ResolvedEvent`] from a
[`NamespaceResolver`] which is fed by a [`RawParser`] guarantees that
the XML document has been well-formed and namespace-well-formed up to this
point (for the caveats about observing a [`RawEvent`], see [`RawParser`]).

Each event has [`EventMetrics`] attached which give information about the
number of bytes from the input stream used to generate the event.

   [`RawParser`]: crate::RawParser
*/
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ResolvedEvent {
	/// The XML declaration.
	///
	/// This mirrors [`RawEvent::XMLDeclaration`].
	XMLDeclaration(
		/// Number of bytes contributing to this event.
		///
		/// This includes all bytes from the opening `<?` until and including
		/// the closing `?>`.
		EventMetrics,
		/// XML version number
		XMLVersion,
	),
	/// The start of an XML element.
	StartElement(
		/// Number of bytes contributing to this event.
		///
		/// If this is the root element, this also includes any whitespace
		/// between the XML declaration and the start of the root element.
		EventMetrics,
		/// The namespace URI / localpart pair of the element.
		ResolvedQName,
		/// Attributes declared on the element, without XML namespace
		/// declarations.
		HashMap<ResolvedQName, CData>,
	),
	/// The end of an XML element.
	///
	/// The parser enforces that start/end pairs are correctly nested.
	EndElement(
		/// Number of bytes contributing to this event.
		///
		/// The number of bytes may be zero if this event is emitted in
		/// response to a `/>` in an element header, because the bytes for
		/// `/>` are accounted for in the corresponding
		/// [`Self::StartElement`].
		EventMetrics,
	),

	/// Text CData.
	///
	/// This mirrors [`RawEvent::Text`].
	///
	/// **Note:** Multiple consecutive `Text` events may be emitted for long
	/// sections of text or because of implementation details in the
	/// processing.
	Text(
		/// Number of bytes contributing to this event.
		///
		/// Note that due to the expansion of character references and the
		/// processing of CDATA sections, the number of bytes consumed will
		/// generally be not equal to the number of bytes in emitted.
		EventMetrics,
		/// Text content
		///
		/// References are expanded and CDATA sections processed correctly, so
		/// that the text in the event exactly corresponds to the *logical*
		/// character data.
		CData,
	),
}

impl ResolvedEvent {
	/// Return the [`EventMetrics`] of the event
	pub fn metrics(&self) -> &EventMetrics {
		match self {
			Self::XMLDeclaration(m, ..) => &m,
			Self::StartElement(m, ..) => &m,
			Self::EndElement(m, ..) => &m,
			Self::Text(m, ..) => &m,
		}
	}
}

enum State {
	Initial,
	Element,
}

struct ElementScratchpad {
	phyqname: RawQName,
	attributes: Vec<(RawQName, CData)>,
	default_decl: Option<NamespaceName>,
	nsdecl: HashMap<NCName, NamespaceName>,
}

impl ElementScratchpad {
	fn new(phyqname: RawQName) -> Self {
		Self {
			phyqname,
			attributes: Vec::new(),
			default_decl: None,
			nsdecl: HashMap::new(),
		}
	}
}

/**
# Namespace/Attribute resolver

This struct implements the resolution logic to convert namespace prefixes into
namespace names (URIs), as described in Namespaces for XML 1.0. It takes
[`RawEvent`] structs and combines/converts them into [`ResolvedEvent`]
structs.

## Caveat

This struct does *not* validate that the sequence of [`RawEvent`] structs it
is fed is actually a well-formed XML document. For instance, it will happily
forward a [`RawEvent::Text`] right after a [`RawEvent::ElementHeadOpen`].
*/
pub struct NamespaceResolver {
	ctx: RcPtr<context::Context>,
	fixed_xml_namespace: NamespaceName,
	namespace_stack: Vec<(Option<NamespaceName>, HashMap<NCName, NamespaceName>)>,
	scratchpad: Option<ElementScratchpad>,
	event_length_accum: usize,
	state: State,
	poison: Option<Error>,
}

impl NamespaceResolver {
	/// Create a new namespace resolver with its own (unshared)
	/// [`context::Context`].
	pub fn new() -> Self {
		Self::with_context(RcPtr::new(context::Context::new()))
	}

	/// Create a new namespace resolver with the given [`context::Context`].
	pub fn with_context(ctx: RcPtr<context::Context>) -> Self {
		let fixed_xml_namespace = ctx.intern_cdata(Cow::Borrowed(XMLNS_XML));
		Self {
			ctx,
			fixed_xml_namespace,
			namespace_stack: Vec::new(),
			scratchpad: None,
			event_length_accum: 0,
			state: State::Initial,
			poison: None,
		}
	}

	fn check_poison(&self) -> Result<()> {
		if let Some(poison) = self.poison.as_ref() {
			return Err(poison.clone());
		}
		Ok(())
	}

	fn start_element(&mut self, phyqn: RawQName) -> Result<()> {
		assert!(self.scratchpad.is_none());
		self.scratchpad = Some(ElementScratchpad::new(phyqn));
		Ok(())
	}

	fn push_attribute(&mut self, phyqn: RawQName, value: CData) -> Result<()> {
		let scratchpad = self.scratchpad.as_mut().unwrap();
		if let Some(prefix) = phyqn.0.as_ref() {
			if prefix == "xmlns" {
				match scratchpad.nsdecl.entry(phyqn.1) {
					// XML 1.0
					// Well-formedness constraint: Unique Att Spec
					Entry::Occupied(_) => {
						return Err(Error::NotWellFormed(WFError::DuplicateAttribute))
					}
					Entry::Vacant(e) => e.insert(self.ctx.intern_cdata(value)),
				};
				return Ok(());
			}
		} else if phyqn.1 == "xmlns" {
			scratchpad.default_decl = Some(self.ctx.intern_cdata(value));
			return Ok(());
		}
		scratchpad.attributes.push((phyqn, value));
		Ok(())
	}

	fn lookup_prefix<'x>(&self, prefix: Option<&'x str>) -> Result<Option<&NamespaceName>> {
		match prefix {
			None => {
				for (default_decl, _) in self.namespace_stack.iter().rev() {
					if let Some(nsuri) = default_decl.as_ref() {
						if nsuri.len() > 0 {
							return Ok(Some(nsuri));
						} else {
							return Ok(None);
						}
					}
				}
				Ok(None)
			}
			Some(prefix) => {
				if prefix == "xml" {
					return Ok(Some(&self.fixed_xml_namespace));
				} else {
					for (_, decls) in self.namespace_stack.iter().rev() {
						if let Some(nsuri) = decls.get(prefix) {
							return Ok(Some(nsuri));
						}
					}
				}
				// Namespaces for XML 1.0
				// Namespace constraint: Prefix Declared
				Err(Error::NotNamespaceWellFormed(
					NWFError::UndeclaredNamespacePrefix(errctx::ERRCTX_UNKNOWN),
				))
			}
		}
	}

	fn finish_element(&mut self) -> Result<ResolvedEvent> {
		let ElementScratchpad {
			phyqname,
			attributes: mut phyattributes,
			default_decl,
			nsdecl,
		} = self.scratchpad.take().unwrap();
		let len = self.event_length_accum;
		self.event_length_accum = 0;

		self.namespace_stack.push((default_decl, nsdecl));

		let mut attributes = HashMap::with_capacity(phyattributes.len());
		for (phyqn, value) in phyattributes.drain(..) {
			let nsuri = match phyqn.0 {
				Some(prefix) => {
					add_context(self.lookup_prefix(Some(&prefix)), errctx::ERRCTX_ATTNAME)?.cloned()
				}
				None => None,
			};
			let qn = (nsuri, phyqn.1);
			match attributes.entry(qn) {
				// XML 1.0
				// Well-formedness constraint: Unique Att Spec
				// Namespaces in XML 1.0
				// Namespace constraint: Attributes Unique
				// We cannot distinguish between the two violations at this point anymore, and the difference is in most cases irrelevant, so we don't.
				Entry::Occupied(_) => {
					return Err(Error::NotWellFormed(WFError::DuplicateAttribute))
				}
				Entry::Vacant(e) => e.insert(value),
			};
		}

		let qname = (
			add_context(
				self.lookup_prefix(phyqname.0.as_ref().map(|x| x.as_str())),
				errctx::ERRCTX_NAME,
			)?
			.cloned(),
			phyqname.1,
		);
		Ok(ResolvedEvent::StartElement(
			EventMetrics { len },
			qname,
			attributes,
		))
	}

	fn process_event(&mut self, ev: RawEvent) -> Result<Option<ResolvedEvent>> {
		// returning Ok(None) does not signal EOF here, but "read more"
		match ev {
			RawEvent::ElementHeadOpen(_, phyqn) => match self.state {
				State::Initial => {
					self.state = State::Element;
					self.start_element(phyqn)?;
					Ok(None)
				}
				_ => unreachable!(),
			},
			RawEvent::Attribute(_, phyqn, value) => match self.state {
				State::Element => {
					self.push_attribute(phyqn, value)?;
					Ok(None)
				}
				_ => unreachable!(),
			},
			RawEvent::ElementHeadClose(_) => match self.state {
				State::Element => {
					let ev = self.finish_element()?;
					self.state = State::Initial;
					Ok(Some(ev))
				}
				_ => unreachable!(),
			},
			RawEvent::ElementFoot(em) => {
				self.namespace_stack.pop();
				Ok(Some(ResolvedEvent::EndElement(em)))
			}
			RawEvent::XMLDeclaration(em, v) => {
				self.event_length_accum = 0;
				Ok(Some(ResolvedEvent::XMLDeclaration(em, v)))
			}
			RawEvent::Text(em, v) => {
				self.event_length_accum = 0;
				Ok(Some(ResolvedEvent::Text(em, v)))
			}
		}
	}

	/// Read [`RawEvent`] structs from the given function until either an
	/// error occurs or a valid [`ResolvedEvent`] can be emitted.
	///
	/// If the [`NamespaceResolver`] detects an error (such as a duplicate
	/// attribute), that error will henceforth be returned whenever this
	/// function is called, no matter the `f`; the `NamespaceResolver` is then
	/// poisoned.
	///
	/// Errors from `f` are forwarded, but do not poison the
	/// [`NamespaceResolver`].
	pub fn next<F: FnMut() -> Result<Option<RawEvent>>>(
		&mut self,
		mut f: F,
	) -> Result<Option<ResolvedEvent>> {
		self.check_poison()?;
		loop {
			let pev = match f() {
				Ok(None) => return Ok(None),
				Err(e) => return Err(e),
				Ok(Some(pev)) => pev,
			};
			self.event_length_accum += pev.metrics().len();
			match self.process_event(pev) {
				Err(e) => {
					self.poison = Some(e.clone());
					return Err(e);
				}
				Ok(Some(v)) => return Ok(Some(v)),
				// None does not signal EOF here, but "read more"
				Ok(None) => (),
			}
		}
	}

	/// Access the inner context
	pub fn context(&self) -> &RcPtr<context::Context> {
		&self.ctx
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::convert::TryInto;

	const DM: EventMetrics = EventMetrics { len: 0 };

	fn resolve_all(mut evs: Vec<RawEvent>) -> (Vec<ResolvedEvent>, Result<()>) {
		let mut nsr = NamespaceResolver::new();
		let mut out = Vec::new();
		let mut iter = evs.drain(..);
		loop {
			match nsr.next(|| Ok(iter.next())) {
				Err(err) => return (out, Err(err)),
				Ok(Some(ev)) => out.push(ev),
				Ok(None) => return (out, Ok(())),
			}
		}
	}

	#[test]
	fn namespace_resolver_passes_xml_decl() {
		let (evs, r) = resolve_all(vec![RawEvent::XMLDeclaration(
			EventMetrics { len: 2342 },
			XMLVersion::V1_0,
		)]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::XMLDeclaration(em, v) => {
				assert_eq!(em.len(), 2342);
				assert_eq!(*v, XMLVersion::V1_0);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_aggregates_attributes_and_length() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 2 }, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(None, "a2".try_into().unwrap()),
				"v2".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert!(nsuri.is_none());
				assert_eq!(localpart, "root");
				assert_eq!(attrs.get(&(None, "a1".try_into().unwrap())).unwrap(), "v1");
				assert_eq!(attrs.get(&(None, "a2".try_into().unwrap())).unwrap(), "v2");
				assert_eq!(attrs.len(), 2);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 6);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_passes_mixed_content() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 1 }, (None, "root".try_into().unwrap())),
			RawEvent::ElementHeadClose(EventMetrics { len: 2 }),
			RawEvent::Text(EventMetrics { len: 5 }, "Hello".try_into().unwrap()),
			RawEvent::ElementHeadOpen(EventMetrics { len: 1 }, (None, "child".try_into().unwrap())),
			RawEvent::ElementHeadClose(EventMetrics { len: 3 }),
			RawEvent::Text(EventMetrics { len: 6 }, "mixed".try_into().unwrap()),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
			RawEvent::Text(EventMetrics { len: 7 }, "world!".try_into().unwrap()),
			RawEvent::ElementFoot(EventMetrics { len: 8 }),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 3);
				assert!(nsuri.is_none());
				assert_eq!(localpart, "root");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::Text(em, text) => {
				assert_eq!(em.len(), 5);
				assert_eq!(text, "Hello");
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 4);
				assert!(nsuri.is_none());
				assert_eq!(localpart, "child");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::Text(em, text) => {
				assert_eq!(em.len(), 6);
				assert_eq!(text, "mixed");
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 6);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::Text(em, text) => {
				assert_eq!(em.len(), 7);
				assert_eq!(text, "world!");
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 8);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_rejects_duplicate_attribute_name() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 2 }, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(None, "a1".try_into().unwrap()),
				"v2".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		match r {
			Err(Error::NotWellFormed(WFError::DuplicateAttribute)) => (),
			other => panic!("unexpected result: {:?}", other),
		}
		let mut iter = evs.iter();
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_returns_error_forever() {
		let pevs_invalid = vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 2 }, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(None, "a1".try_into().unwrap()),
				"v2".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		];
		let pevs_valid = vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 2 }, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		];
		let mut nsr = NamespaceResolver::new();
		{
			let mut iter = pevs_invalid.iter();
			match nsr.next(|| Ok(iter.next().cloned())) {
				Err(Error::NotWellFormed(WFError::DuplicateAttribute)) => (),
				other => panic!("unexpected result: {:?}", other),
			}
		}
		{
			let mut iter = pevs_valid.iter();
			match nsr.next(|| Ok(iter.next().cloned())) {
				Err(Error::NotWellFormed(WFError::DuplicateAttribute)) => (),
				other => panic!("unexpected result: {:?}", other),
			}
		}
	}

	#[test]
	fn namespace_resolver_resolves_default_namespace_on_element() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 2 }, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(None, "xmlns".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "root");
				assert_eq!(attrs.get(&(None, "a1".try_into().unwrap())).unwrap(), "v1");
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 6);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_resolves_prefixed_namespace_on_element() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 2 },
				(Some("foo".try_into().unwrap()), "root".try_into().unwrap()),
			),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(Some("xmlns".try_into().unwrap()), "foo".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "root");
				assert_eq!(attrs.get(&(None, "a1".try_into().unwrap())).unwrap(), "v1");
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 6);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_resolves_prefixed_namespace_on_attribute() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(EventMetrics { len: 2 }, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(Some("foo".try_into().unwrap()), "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(Some("xmlns".try_into().unwrap()), "foo".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert!(nsuri.is_none());
				assert_eq!(localpart, "root");
				assert_eq!(
					attrs
						.get(&(
							Some(RcPtr::new("foo".try_into().unwrap())),
							"a1".try_into().unwrap()
						))
						.unwrap(),
					"v1"
				);
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 6);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_resolves_prefixed_namespace_on_nested_elements() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 2 },
				(Some("x".try_into().unwrap()), "root".try_into().unwrap()),
			),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 1 },
				(Some("x".try_into().unwrap()), "child".try_into().unwrap()),
			),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(Some("x".try_into().unwrap()), "a2".try_into().unwrap()),
				"v2".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 2 }),
			RawEvent::ElementFoot(EventMetrics { len: 4 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "root");
				assert_eq!(attrs.get(&(None, "a1".try_into().unwrap())).unwrap(), "v1");
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 6);
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "child");
				assert_eq!(
					attrs
						.get(&(
							Some(RcPtr::new("foo".try_into().unwrap())),
							"a2".try_into().unwrap()
						))
						.unwrap(),
					"v2"
				);
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 4);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(em) => {
				assert_eq!(em.len(), 6);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_rejects_undeclared_prefix_in_element_name() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 2 },
				(Some("x".try_into().unwrap()), "root".try_into().unwrap()),
			),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 1 },
				(Some("foo".try_into().unwrap()), "child".try_into().unwrap()),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 2 }),
			RawEvent::ElementFoot(EventMetrics { len: 4 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "root");
				assert_eq!(attrs.get(&(None, "a1".try_into().unwrap())).unwrap(), "v1");
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match r {
			Err(Error::NotNamespaceWellFormed(NWFError::UndeclaredNamespacePrefix(
				errctx::ERRCTX_NAME,
			))) => (),
			other => panic!("unexpected result: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_rejects_undeclared_prefix_in_attribute_name() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 2 },
				(Some("x".try_into().unwrap()), "root".try_into().unwrap()),
			),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(None, "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				EventMetrics { len: 4 },
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 5 }),
			RawEvent::ElementHeadOpen(
				EventMetrics { len: 1 },
				(Some("x".try_into().unwrap()), "child".try_into().unwrap()),
			),
			RawEvent::Attribute(
				EventMetrics { len: 3 },
				(Some("foo".try_into().unwrap()), "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(EventMetrics { len: 2 }),
			RawEvent::ElementFoot(EventMetrics { len: 4 }),
			RawEvent::ElementFoot(EventMetrics { len: 6 }),
		]);
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(em, (nsuri, localpart), attrs) => {
				assert_eq!(em.len(), 14);
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "root");
				assert_eq!(attrs.get(&(None, "a1".try_into().unwrap())).unwrap(), "v1");
				assert_eq!(attrs.len(), 1);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match r {
			Err(Error::NotNamespaceWellFormed(NWFError::UndeclaredNamespacePrefix(
				errctx::ERRCTX_ATTNAME,
			))) => (),
			other => panic!("unexpected result: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_rejects_duplicate_attribute_post_namespace_resolution() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(DM, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "y".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(DM),
			RawEvent::ElementHeadOpen(DM, (None, "child".try_into().unwrap())),
			RawEvent::Attribute(
				DM,
				(Some("x".try_into().unwrap()), "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("y".try_into().unwrap()), "a1".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(DM),
			RawEvent::ElementFoot(DM),
			RawEvent::ElementFoot(DM),
		]);
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(_, (nsuri, localpart), attrs) => {
				assert!(nsuri.is_none());
				assert_eq!(localpart, "root");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match r {
			Err(Error::NotWellFormed(WFError::DuplicateAttribute)) => (),
			other => panic!("unexpected result: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_rejects_namespace_redeclaration_within_the_same_header() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(DM, (None, "root".try_into().unwrap())),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(DM),
			RawEvent::ElementFoot(DM),
		]);
		let mut iter = evs.iter();
		match r {
			Err(Error::NotWellFormed(WFError::DuplicateAttribute)) => (),
			other => panic!("unexpected result: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}

	#[test]
	fn namespace_resolver_with_multiple_prefixes_and_rebinding() {
		let (evs, r) = resolve_all(vec![
			RawEvent::ElementHeadOpen(
				DM,
				(Some("x".try_into().unwrap()), "root".try_into().unwrap()),
			),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "x".try_into().unwrap()),
				"foo".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "y".try_into().unwrap()),
				"bar".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("x".try_into().unwrap()), "a".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("y".try_into().unwrap()), "a".try_into().unwrap()),
				"v2".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(DM),
			RawEvent::ElementHeadOpen(
				DM,
				(Some("y".try_into().unwrap()), "child".try_into().unwrap()),
			),
			RawEvent::Attribute(
				DM,
				(Some("x".try_into().unwrap()), "a".try_into().unwrap()),
				"v1".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("y".try_into().unwrap()), "a".try_into().unwrap()),
				"v2".try_into().unwrap(),
			),
			RawEvent::Attribute(
				DM,
				(Some("xmlns".try_into().unwrap()), "y".try_into().unwrap()),
				"baz".try_into().unwrap(),
			),
			RawEvent::ElementHeadClose(DM),
			RawEvent::ElementFoot(DM),
			RawEvent::ElementFoot(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(_, (nsuri, localpart), attrs) => {
				assert_eq!(**nsuri.as_ref().unwrap(), "foo");
				assert_eq!(localpart, "root");
				assert_eq!(
					attrs
						.get(&(
							Some(RcPtr::new("foo".try_into().unwrap())),
							"a".try_into().unwrap()
						))
						.unwrap(),
					"v1"
				);
				assert_eq!(
					attrs
						.get(&(
							Some(RcPtr::new("bar".try_into().unwrap())),
							"a".try_into().unwrap()
						))
						.unwrap(),
					"v2"
				);
				assert_eq!(attrs.len(), 2);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::StartElement(_, (nsuri, localpart), attrs) => {
				assert_eq!(**nsuri.as_ref().unwrap(), "baz");
				assert_eq!(localpart, "child");
				assert_eq!(
					attrs
						.get(&(
							Some(RcPtr::new("foo".try_into().unwrap())),
							"a".try_into().unwrap()
						))
						.unwrap(),
					"v1"
				);
				assert_eq!(
					attrs
						.get(&(
							Some(RcPtr::new("baz".try_into().unwrap())),
							"a".try_into().unwrap()
						))
						.unwrap(),
					"v2"
				);
				assert_eq!(attrs.len(), 2);
			}
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(_) => (),
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next().unwrap() {
			ResolvedEvent::EndElement(_) => (),
			other => panic!("unexpected event: {:?}", other),
		}
		match iter.next() {
			None => (),
			other => panic!("unexpected event: {:?}", other),
		}
	}
}
