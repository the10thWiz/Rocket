use std::hash::Hash;

use proc_macro2::{TokenStream, Span};
use devise::{Spanned, SpanWrapped, Result, FromMeta, Diagnostic};
use devise::ext::{TypeExt as _, SpanDiagnosticExt};

use crate::{name::Name, proc_macro2, proc_macro_ext::Diagnostics, syn};
use crate::proc_macro_ext::StringLit;
use crate::syn_ext::{IdentExt, TypeExt as _, FnArgExt};
use crate::http_codegen::{Method, Optional};
use crate::attribute::param::Guard;
use crate::attribute::route::parse::RouteUri;

use super::{param::Parameter, route::parse::{ArgumentMap, Arguments}};


