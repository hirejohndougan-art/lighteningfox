use crate::error::RuntimeLimitError;
use crate::{Context, JsResult, vm::opcode::Operation};

/// `IncrementLoopIteration` implements the Opcode Operation for `Opcode::IncrementLoopIteration`.
///
/// Operation:
///  - Increment loop iteration count.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IncrementLoopIteration;

impl IncrementLoopIteration {
    /// Charges one iteration against the loop limit, for native code that loops (such as
    /// `String.prototype.repeat`). Unlike the opcode, it never tiers up or enters compiled
    /// code: the frame on top is whoever called the native function, mid-call.
    pub(crate) fn charge(context: &mut Context) -> JsResult<()> {
        let max = context.vm.runtime_limits.loop_iteration_limit();
        let frame = context.vm.frame_mut();
        let previous_iteration_count = frame.loop_iteration_count;
        if previous_iteration_count > max {
            return Err(RuntimeLimitError::LoopIteration.into());
        }
        frame.loop_iteration_count = previous_iteration_count.wrapping_add(1);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn operation((): (), context: &mut Context) -> JsResult<()> {
        let max = context.vm.runtime_limits.loop_iteration_limit();
        let frame = context.vm.frame_mut();
        let previous_iteration_count = frame.loop_iteration_count;

        if previous_iteration_count > max {
            return Err(RuntimeLimitError::LoopIteration.into());
        }

        frame.loop_iteration_count = previous_iteration_count.wrapping_add(1);

        // Ironswift change: count loop iterations for the baseline JIT (docs/JIT.md).
        #[cfg(feature = "jit")]
        if frame.code_block.jit.note_loop_iteration() {
            let code = frame.code_block.clone();
            crate::jit::tier_up(&code, context);
        }
        // A loop header in a compiled function: go on in machine code.
        #[cfg(feature = "jit")]
        crate::jit::enter(context)?;
        Ok(())
    }
}

impl Operation for IncrementLoopIteration {
    const NAME: &'static str = "IncrementLoopIteration";
    const INSTRUCTION: &'static str = "INST - IncrementLoopIteration";
    const COST: u8 = 3;
}
