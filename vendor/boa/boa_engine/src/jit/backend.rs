//! Cranelift and executable memory.
//!
//! `cranelift-jit` keeps code pages writable while a batch of functions is written and turns
//! them read+execute (never read+write+execute) in `finalize_definitions`. Finalized pages are
//! never written again: later functions go into fresh pages. That is the W^X rule in
//! `docs/JIT.md`; the `executable_memory_is_never_writable` test checks it on Linux.

use cranelift_codegen::ir::{AbiParam, Signature, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Module, default_libcall_names};
use std::fmt;

/// One Cranelift JIT module: an instruction-set target plus the executable memory its
/// functions live in. One per thread (a `Context` never moves between threads).
pub struct JitBackend {
    module: JITModule,
    builder_context: FunctionBuilderContext,
    functions: usize,
}

impl fmt::Debug for JitBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JitBackend")
            .field("isa", &self.module.isa().triple().to_string())
            .field("functions", &self.functions)
            .finish()
    }
}

impl JitBackend {
    /// Sets up Cranelift for the CPU this process runs on.
    ///
    /// # Errors
    ///
    /// Fails when Cranelift has no backend for this CPU.
    pub fn new() -> Result<Self, String> {
        let mut flags = settings::builder();
        for (name, value) in [
            ("opt_level", "speed"),
            ("is_pic", "false"),
            ("use_colocated_libcalls", "false"),
            // Compiled frames must be walkable by debuggers and profilers.
            ("preserve_frame_pointers", "true"),
        ] {
            flags.set(name, value).map_err(|e| e.to_string())?;
        }
        let isa = cranelift_native::builder()
            .map_err(str::to_string)?
            .finish(settings::Flags::new(flags))
            .map_err(|e| e.to_string())?;
        let module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
        Ok(Self {
            module,
            builder_context: FunctionBuilderContext::new(),
            functions: 0,
        })
    }

    /// A signature in the host's calling convention.
    #[must_use]
    pub fn signature(&self, params: &[types::Type], returns: &[types::Type]) -> Signature {
        let mut sig = self.module.make_signature();
        sig.params.extend(params.iter().map(|&t| AbiParam::new(t)));
        sig.returns.extend(returns.iter().map(|&t| AbiParam::new(t)));
        sig
    }

    /// The pointer-sized integer type.
    #[must_use]
    pub fn pointer_type(&self) -> types::Type {
        self.module.target_config().pointer_type()
    }

    /// Compiles one function and returns its entry point, already read+execute.
    ///
    /// `build` gets a fresh builder for a function with signature `sig`; it must create the
    /// blocks, fill them and seal them (the builder is finalized here).
    ///
    /// # Errors
    ///
    /// Fails when Cranelift rejects the function (a verifier error is a compiler bug; the
    /// caller should leave the function to the interpreter).
    pub fn compile(
        &mut self,
        sig: Signature,
        build: impl FnOnce(&mut FunctionBuilder<'_>),
    ) -> Result<*const u8, String> {
        let frontend_config = self.module.target_config();
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_context);
            build(&mut builder);
            builder.finalize(frontend_config);
        }
        let id = self
            .module
            .declare_anonymous_function(&ctx.func.signature)
            .map_err(|e| e.to_string())?;
        let defined = self
            .module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("{e:?}"));
        self.module.clear_context(&mut ctx);
        defined?;
        self.module
            .finalize_definitions()
            .map_err(|e| e.to_string())?;
        self.functions += 1;
        Ok(self.module.get_finalized_function(id))
    }

    /// How many functions this backend has compiled.
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.functions
    }
}
