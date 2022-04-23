/*!
# Macros for XML strings

This crate provides macros to check XML string syntax at compile time.

## Example

```rust,ignore
use rxml::{CDataStr, NCNameStr};
use rxml_proc::*;

const XML_NAMESPACE: &'static CDataStr = xml_cdata!("http://www.w3.org/XML/1998/namespace");
const XML_PREFIX: &'static NCNameStr = xml_ncname!("xml");
```

## See also

This crate bases on the [`rxml_validation`] crate and it primarily intended
for use with the [`rxml`](https://docs.rs/rxml) crate.
*/
use proc_macro::TokenStream;
use quote::quote;
use rxml_validation::{validate_cdata, validate_name, validate_ncname};
use syn::{parse_macro_input, LitStr};

/** XML 1.0 CData compliant string

# Example

```rust,ignore
use rxml::CDataStr;
use rxml_proc::xml_cdata;

const XML_NAMESPACE: &'static CDataStr = xml_cdata!("http://www.w3.org/XML/1998/namespace");
*/
#[proc_macro]
pub fn xml_cdata(input: TokenStream) -> TokenStream {
	let data = parse_macro_input!(input as LitStr);
	let s = data.value();
	let tokens = match validate_cdata(&s) {
		Ok(()) => quote! { unsafe { std::mem::transmute::<_, &rxml::CDataStr>(#s) } },
		Err(e) => {
			let err = format!("invalid CData string {:?}: {}", s, e);
			quote! { compile_error!(#err) }
		}
	};
	tokens.into()
}

/** XML 1.0 Name compliant string

# Example

```rust,ignore
use rxml::NameStr;
use rxml_proc::xml_name;

const FORBIDDEN: &'static NameStr = xml_name!("xmlns:xml");
*/
#[proc_macro]
pub fn xml_name(input: TokenStream) -> TokenStream {
	let data = parse_macro_input!(input as LitStr);
	let s = data.value();
	let tokens = match validate_name(&s) {
		Ok(()) => quote! { unsafe { std::mem::transmute::<_, &rxml::NameStr>(#s) } },
		Err(e) => {
			let err = format!("invalid Name string {:?}: {}", s, e);
			quote! { compile_error!(#err) }
		}
	};
	tokens.into()
}

/** Namespaces for XML 1.0 NCName compliant string

# Example

```rust,ignore
use rxml::NCNameStr;
use rxml_proc::xml_ncname;

const XML_PREFIX: &'static NCNameStr = xml_ncname!("xml");
*/
#[proc_macro]
pub fn xml_ncname(input: TokenStream) -> TokenStream {
	let data = parse_macro_input!(input as LitStr);
	let s = data.value();
	let tokens = match validate_ncname(&s) {
		Ok(()) => quote! { unsafe { std::mem::transmute::<_, &rxml::NcNameStr>(#s) } },
		Err(e) => {
			let err = format!("invalid NCName string {:?}: {}", s, e);
			quote! { compile_error!(#err) }
		}
	};
	tokens.into()
}
