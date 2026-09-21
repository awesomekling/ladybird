/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::bridge::{FfiFontResolutionRequest, FfiResolvedFont};
use super::{HashMap, HashSet};
use crate::css::style_value::{RetainedStyleValueData, retain_style_value, style_value_content_hash};
use libgfx_rust::font::FontCascadeListHandle;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};

pub type ResolveFontsCallback =
    unsafe extern "C" fn(*mut c_void, *const FfiFontResolutionRequest, *mut FfiResolvedFont, usize);

#[derive(Clone, Copy)]
pub(super) enum FontService {
    ParkedBatch,
    RootPreparation,
}

impl FontService {
    fn name(self) -> &'static str {
        match self {
            Self::ParkedBatch => "resolve_font",
            Self::RootPreparation => "resolve_font_root",
        }
    }
}

struct FontResolutionKey {
    font_family: RetainedStyleValueData,
    font_size_raw: i32,
    font_slope: i32,
    font_weight: u64,
    font_width: u64,
    font_optical_sizing: u8,
}

impl FontResolutionKey {
    fn new(request: FfiFontResolutionRequest) -> Self {
        Self {
            font_family: unsafe {
                RetainedStyleValueData::from_retained_pointer(retain_style_value(
                    request.font_family.as_pointer().cast(),
                ))
            },
            font_size_raw: request.font_size_raw,
            font_slope: request.font_slope,
            font_weight: request.font_weight.to_bits(),
            font_width: request.font_width.to_bits(),
            font_optical_sizing: request.font_optical_sizing,
        }
    }
}

impl PartialEq for FontResolutionKey {
    fn eq(&self, other: &Self) -> bool {
        self.font_family == other.font_family
            && self.font_size_raw == other.font_size_raw
            && self.font_slope == other.font_slope
            && self.font_weight == other.font_weight
            && self.font_width == other.font_width
            && self.font_optical_sizing == other.font_optical_sizing
    }
}

impl Eq for FontResolutionKey {}

impl Hash for FontResolutionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        unsafe { style_value_content_hash(self.font_family.pointer()) }.hash(state);
        self.font_size_raw.hash(state);
        self.font_slope.hash(state);
        self.font_weight.hash(state);
        self.font_width.hash(state);
        self.font_optical_sizing.hash(state);
    }
}

/// Own the request's family until the boundary transfers it into the prepared table.
pub(super) struct FontRequest {
    ffi: FfiFontResolutionRequest,
    family: RetainedStyleValueData,
}

impl FontRequest {
    pub fn new(ffi: FfiFontResolutionRequest) -> Self {
        let family = unsafe {
            RetainedStyleValueData::from_retained_pointer(retain_style_value(ffi.font_family.as_pointer().cast()))
        };
        Self { ffi, family }
    }

    pub fn for_generation(&self, generation: u64) -> Self {
        let mut ffi = self.ffi;
        ffi.font_environment_generation = generation;
        Self {
            ffi,
            family: self.family.clone(),
        }
    }
}

/// One reference to a host `Gfx::FontCascadeList`, held so the list the engine names stays alive
/// while a resolution names it.
struct SharedFontCascadeList(#[expect(dead_code, reason = "held for the reference it owns")] FontCascadeListHandle);

// SAFETY: An evaluation step reaches this type only through `&FontResolutionCache`, and `lookup`
// copies the `FfiResolvedFont` out without ever naming the handle, so no worker can move or drop
// one. That is exactly `Sync` and deliberately not `Send`: the handle is borrowed by a walk,
// never given to it. What matters is which thread performs the *final* release, because that runs
// `~FontCascadeList`. This handle is the reason it is never a worker: the cache holds one reference
// per cached resolution for the whole font-environment generation, taken here and given up only in
// `FontResolutionCache::prepare` or when the cache is dropped - a host round on the engine's thread.
// A step that builds a font style group takes a second reference to the same list
// (`build_font_group` in `table_group_builder.rs`) and may give it up again when the rebuilt payload
// is canonicalized away. `Gfx::FontCascadeList`, `Gfx::Font`, and `Gfx::Typeface` are all atomically
// reference-counted, so that pair is safe. Keeping final destruction on the host also keeps it away
// from concurrently used mutable font and typeface caches.
unsafe impl Sync for SharedFontCascadeList {}

struct ResolvedFont {
    _font_cascade_list: Option<SharedFontCascadeList>,
    ffi: FfiResolvedFont,
}

/// The resolutions this document has already been given, keyed by content and scoped to one
/// font-environment generation. This is retained engine state: an evaluation step reads it and
/// never calls the host.
#[derive(Default)]
pub(super) struct FontResolutionCache {
    generation: Option<u64>,
    cache: HashMap<FontResolutionKey, ResolvedFont>,
}

impl FontResolutionCache {
    pub fn prepare(&mut self, generation: u64) {
        if self.generation != Some(generation) {
            self.cache.clear();
            self.generation = Some(generation);
        }
    }

    pub fn lookup(&self, request: FfiFontResolutionRequest) -> Option<FfiResolvedFont> {
        if self.generation != Some(request.font_environment_generation) {
            return None;
        }
        self.cache
            .get(&FontResolutionKey::new(request))
            .map(|resolved| resolved.ffi)
    }

    fn insert(&mut self, request: FontRequest, ffi: FfiResolvedFont) {
        // A null result is a completed, unsupported host resolution, not another cache miss.
        let font_cascade_list = (!ffi.font_cascade_list.is_none()).then(|| {
            // SAFETY: The callback transfers one reference to a live list.
            SharedFontCascadeList(unsafe { FontCascadeListHandle::adopt(ffi.font_cascade_list.as_pointer()) })
        });
        self.cache.insert(
            FontResolutionKey::new(request.ffi),
            ResolvedFont {
                _font_cascade_list: font_cascade_list,
                ffi,
            },
        );
    }
}

/// The host's synchronous font resolver. This is host state: it holds a C++ context pointer and
/// the callback into it, and only a round between evaluation passes may call it.
pub(super) struct FontResolverHost {
    context: *mut c_void,
    resolve: ResolveFontsCallback,
}

impl FontResolverHost {
    pub fn new(context: *mut c_void, resolve: ResolveFontsCallback) -> Self {
        Self { context, resolve }
    }

    /// Service synchronous requests in one round between evaluation passes. Pending web faces
    /// remain in the returned cascades and retain the host's rendering-triggered loading behavior.
    pub fn refill(
        &self,
        cache: &mut FontResolutionCache,
        mut requests: Vec<FontRequest>,
        service: FontService,
    ) -> usize {
        let Some(first) = requests.first() else {
            return 0;
        };
        let generation = first.ffi.font_environment_generation;
        debug_assert!(
            requests
                .iter()
                .all(|request| request.ffi.font_environment_generation == generation)
        );
        cache.prepare(generation);
        let mut unique = HashSet::default();
        requests.retain(|request| {
            cache.lookup(request.ffi).is_none() && unique.insert(FontResolutionKey::new(request.ffi))
        });
        if requests.is_empty() {
            return 0;
        }
        let ffi_requests = requests.iter().map(|request| request.ffi).collect::<Vec<_>>();
        let mut resolved = vec![FfiResolvedFont::default(); requests.len()];
        super::seal::between_pass_font_service(service.name(), requests.len() as u64, || unsafe {
            (self.resolve)(
                self.context,
                ffi_requests.as_ptr(),
                resolved.as_mut_ptr(),
                requests.len(),
            );
        });
        let count = requests.len();
        for (request, resolved) in requests.into_iter().zip(resolved) {
            cache.insert(request, resolved);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::style_compute::ffi_test_stubs::font_cascade_list_unref_count;
    use crate::css::style_value::StyleValueData;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RESOLVES: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn resolve_fonts(
        _context: *mut c_void,
        _requests: *const FfiFontResolutionRequest,
        resolved: *mut FfiResolvedFont,
        count: usize,
    ) {
        RESOLVES.fetch_add(count, Ordering::Relaxed);
        for index in 0..count {
            unsafe {
                resolved.add(index).write(FfiResolvedFont {
                    first_available_font: crate::css::style::bridge::FfiHostHandle::from_pointer(std::ptr::dangling()),
                    font_cascade_list: crate::css::style::bridge::FfiHostHandle::from_pointer(std::ptr::dangling()),
                    ..Default::default()
                });
            }
        }
    }

    #[test]
    fn font_resolution_cache_is_engine_owned_and_generation_scoped() {
        RESOLVES.store(0, Ordering::Relaxed);
        let unrefs_before = font_cascade_list_unref_count();
        let family = RetainedStyleValueData::from_owned(StyleValueData::Number { value: 1.5 });
        let host = FontResolverHost::new(std::ptr::null_mut(), resolve_fonts);
        let mut resolver = FontResolutionCache::default();
        let mut request = FfiFontResolutionRequest {
            font_family: crate::css::style::bridge::FfiHostHandle::from_pointer(family.pointer().cast()),
            font_size_raw: 1024,
            font_slope: 0,
            font_weight: 400.0,
            font_width: 100.0,
            font_optical_sizing: 0,
            font_environment_generation: 1,
        };

        resolver.prepare(1);
        assert!(resolver.lookup(request).is_none());
        assert_eq!(RESOLVES.load(Ordering::Relaxed), 0);
        host.refill(&mut resolver, vec![FontRequest::new(request)], FontService::ParkedBatch);
        let first = resolver.lookup(request).unwrap();
        assert_eq!(
            resolver.lookup(request).unwrap().font_cascade_list,
            first.font_cascade_list
        );
        assert_eq!(RESOLVES.load(Ordering::Relaxed), 1);
        assert_eq!(font_cascade_list_unref_count(), unrefs_before);

        let equivalent_family = RetainedStyleValueData::from_owned(StyleValueData::Number { value: 1.5 });
        assert_ne!(family.pointer(), equivalent_family.pointer());
        let equivalent_request = FfiFontResolutionRequest {
            font_family: crate::css::style::bridge::FfiHostHandle::from_pointer(equivalent_family.pointer().cast()),
            ..request
        };
        assert_eq!(
            resolver.lookup(equivalent_request).unwrap().font_cascade_list,
            first.font_cascade_list
        );
        assert_eq!(RESOLVES.load(Ordering::Relaxed), 1);

        request.font_environment_generation = 2;
        assert!(resolver.lookup(request).is_none());
        resolver.prepare(2);
        assert_eq!(font_cascade_list_unref_count(), unrefs_before + 1);
        host.refill(&mut resolver, vec![FontRequest::new(request)], FontService::ParkedBatch);
        resolver.lookup(request).unwrap();
        assert_eq!(RESOLVES.load(Ordering::Relaxed), 2);
        assert_eq!(font_cascade_list_unref_count(), unrefs_before + 1);

        drop(resolver);
        assert_eq!(font_cascade_list_unref_count(), unrefs_before + 2);
    }

    #[test]
    fn unavailable_resolution_is_a_completed_answer_until_the_environment_changes() {
        unsafe extern "C" fn unavailable(
            _: *mut c_void,
            _: *const FfiFontResolutionRequest,
            resolved: *mut FfiResolvedFont,
            count: usize,
        ) {
            for index in 0..count {
                unsafe { resolved.add(index).write(FfiResolvedFont::default()) };
            }
        }
        let family = RetainedStyleValueData::from_owned(StyleValueData::Keyword { keyword: 1 });
        let request = FfiFontResolutionRequest {
            font_family: crate::css::style::bridge::FfiHostHandle::from_pointer(family.pointer().cast()),
            font_size_raw: 1024,
            font_slope: 0,
            font_weight: 400.0,
            font_width: 100.0,
            font_optical_sizing: 0,
            font_environment_generation: 1,
        };
        let host = FontResolverHost::new(std::ptr::null_mut(), unavailable);
        let mut resolver = FontResolutionCache::default();
        resolver.prepare(1);
        let owned = FontRequest::new(request);
        drop(family);
        assert!(resolver.lookup(request).is_none());
        host.refill(&mut resolver, vec![owned], FontService::ParkedBatch);
        assert!(resolver.lookup(request).unwrap().font_cascade_list.is_none());
        assert!(resolver.lookup(request).unwrap().font_cascade_list.is_none());
        // A failed synchronous result must not cause an endless refill loop.
        let next = FfiFontResolutionRequest {
            font_environment_generation: 2,
            ..request
        };
        assert!(resolver.lookup(next).is_none());
        resolver.prepare(2);
    }
}
