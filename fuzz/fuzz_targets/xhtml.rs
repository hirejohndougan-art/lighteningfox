#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| ironswift_fuzz::xhtml(data));
