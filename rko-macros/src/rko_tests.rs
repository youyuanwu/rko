// SPDX-License-Identifier: GPL-2.0

//! `#[rko_tests]` proc macro implementation.
//!
//! Transforms a module containing `#[test]` functions into a self-contained
//! test suite with structured output compatible with the QEMU test runner.
//!
//! See `docs/design/features/test-framework.md`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, Ident, Item, ItemMod, Result, ReturnType};

pub(crate) fn rko_tests(mut module: ItemMod) -> Result<TokenStream> {
    let Some((brace, items)) = module.content.take() else {
        return Err(Error::new_spanned(
            module,
            "`#[rko_tests]` should only be applied to inline modules",
        ));
    };

    let module_name = module.ident.to_string();
    let mut processed: Vec<Item> = Vec::new();
    let mut tests: Vec<Ident> = Vec::new();

    // Shadow `assert!`, `assert_eq!`, and `assert_ne!` with versions that
    // print structured FAIL output and return Err from the test function.
    processed.push(syn::parse_quote! {
        #[allow(unused_macros)]
        macro_rules! assert {
            ($cond:expr $(,)?) => {{
                if !$cond {
                    ::rko_core::pr_err!(
                        "  FAIL: {}, {}:{}\n",
                        stringify!($cond), file!(), line!()
                    );
                    return Err(::rko_core::error::Error::EINVAL);
                }
            }};
        }
    });

    processed.push(syn::parse_quote! {
        #[allow(unused_macros)]
        macro_rules! assert_eq {
            ($left:expr, $right:expr $(,)?) => {{
                match (&$left, &$right) {
                    (left_val, right_val) => {
                        if !(*left_val == *right_val) {
                            ::rko_core::pr_err!(
                                "  FAIL: {} == {}, {}:{}\n",
                                stringify!($left), stringify!($right), file!(), line!()
                            );
                            return Err(::rko_core::error::Error::EINVAL);
                        }
                    }
                }
            }};
        }
    });

    processed.push(syn::parse_quote! {
        #[allow(unused_macros)]
        macro_rules! assert_ne {
            ($left:expr, $right:expr $(,)?) => {{
                match (&$left, &$right) {
                    (left_val, right_val) => {
                        if *left_val == *right_val {
                            ::rko_core::pr_err!(
                                "  FAIL: {} != {}, {}:{}\n",
                                stringify!($left), stringify!($right), file!(), line!()
                            );
                            return Err(::rko_core::error::Error::EINVAL);
                        }
                    }
                }
            }};
        }
    });

    for item in items {
        let Item::Fn(mut f) = item else {
            processed.push(item);
            continue;
        };

        // Check for and strip `#[test]` attribute.
        let before = f.attrs.len();
        f.attrs.retain(|attr| !attr.path().is_ident("test"));
        if f.attrs.len() == before {
            // Not a test function — keep as-is.
            processed.push(Item::Fn(f));
            continue;
        }

        tests.push(f.sig.ident.clone());

        // Functions returning () are transformed to -> Result<(), Error>
        // with Ok(()) appended, so the assert macros can `return Err(...)`.
        if matches!(f.sig.output, ReturnType::Default) {
            f.sig.output = syn::parse_quote!(-> Result<(), ::rko_core::error::Error>);
            let block = &f.block;
            f.block = syn::parse_quote!({
                #block;
                #[allow(unreachable_code)]
                Ok(())
            });
        }

        processed.push(Item::Fn(f));
    }

    // Build the `pub fn run()` test runner.
    let num_tests = tests.len();
    let test_strs: Vec<String> = tests.iter().map(|t| t.to_string()).collect();

    let runners: Vec<TokenStream> = tests
        .iter()
        .zip(test_strs.iter())
        .map(|(ident, name)| {
            quote! {
                match #ident() {
                    Ok(()) => {
                        pass += 1;
                        ::rko_core::pr_info!("  PASS: {}\n", #name);
                    }
                    Err(_) => {
                        fail += 1;
                    }
                }
            }
        })
        .collect();

    processed.push(syn::parse_quote! {
        /// Run all `#[test]` functions in this module.
        pub fn run() -> Result<(), ::rko_core::error::Error> {
            ::rko_core::pr_info!("---- {} ({} tests) ----\n", #module_name, #num_tests);
            let mut pass = 0u32;
            let mut fail = 0u32;
            #(#runners)*
            ::rko_core::pr_info!(
                "---- {}: {} passed, {} failed ----\n",
                #module_name, pass, fail
            );
            if fail > 0 {
                Err(::rko_core::error::Error::EINVAL)
            } else {
                Ok(())
            }
        }
    });

    // Phase 2: KUnit suite registration for automatic discovery.
    //
    // Generate:
    //   unsafe extern "C" fn kunit_rust_wrapper_<test>(_test: *mut c_void)
    //   static mut KUNIT_TEST_CASES: [kunit_case; N+1]
    //   kunit_unsafe_test_suite!(suite_name, KUNIT_TEST_CASES)

    let kunit_wrapper_names: Vec<Ident> = tests
        .iter()
        .map(|t| format_ident!("kunit_rust_wrapper_{}", t))
        .collect();

    for (wrapper, test) in kunit_wrapper_names.iter().zip(tests.iter()) {
        processed.push(syn::parse_quote! {
            #[allow(non_snake_case)]
            unsafe extern "C" fn #wrapper(
                _test: *mut core::ffi::c_void,
            ) {
                if #test().is_err() {
                    unsafe { ::rko_core::kunit::kunit_mark_failed(_test); }
                }
            }
        });
    }

    let num_cases_plus_1 = tests.len() + 1;
    let kunit_case_entries: Vec<TokenStream> = kunit_wrapper_names
        .iter()
        .zip(test_strs.iter())
        .map(|(wrapper, name)| {
            let cstr = format!("{}\0", name);
            quote! {
                ::rko_core::kunit::new_kunit_case(
                    #cstr.as_ptr().cast(),
                    #wrapper,
                )
            }
        })
        .collect();

    let suite_ident = format_ident!("{}", module_name);

    processed.push(syn::parse_quote! {
        #[allow(non_upper_case_globals)]
        static mut KUNIT_TEST_CASES: [::rko_core::kunit::kunit_case; #num_cases_plus_1] = [
            #(#kunit_case_entries,)*
            ::rko_core::kunit::kunit_case_null(),
        ];
    });

    processed.push(syn::parse_quote! {
        ::rko_core::kunit_unsafe_test_suite!(#suite_ident, KUNIT_TEST_CASES);
    });

    module.content = Some((brace, processed));
    Ok(quote! { #module })
}
