use boa_string::StaticJsStrings;

use crate::JsObject;
use crate::object::internal_methods::ORDINARY_INTERNAL_METHODS;
use crate::object::shape::{shape_epoch, slot::Slot};
use crate::vm::inline_cache::{ChainLink, DeepEntry, InlineCache};

use crate::{
    Context, JsResult, JsValue, js_string,
    object::{internal_methods::InternalMethodPropertyContext, shape::slot::SlotAttributes},
    property::PropertyKey,
    vm::opcode::{IndexOperand, Operation, RegisterOperand},
};

fn get_by_name<const LENGTH: bool>(
    (dst, object, receiver, index): (RegisterOperand, &JsValue, &JsValue, IndexOperand),
    context: &mut Context,
) -> JsResult<()> {
    if LENGTH {
        if let Some(object) = object.as_object_ref()
            && object.is_array()
        {
            let value = object.borrow().properties().storage[0].clone();
            context.vm.set_register(dst.into(), value);
            return Ok(());
        } else if let Some(string) = object.as_string() {
            // NOTE: Since we’re using the prototype returned directly by `base_class()`,
            //       we need to handle string primitives separately due to the
            //       string exotic internal methods.
            context
                .vm
                .set_register(dst.into(), (string.len() as u32).into());
            return Ok(());
        }
    }

    // OPTIMIZATION:
    //    Instead of calling `to_object()`, which creates a temporary wrapper object for primitive
    //    values (e.g., numbers, strings, booleans) just to query their prototype chain.
    //
    //    To prevent the creation of a temporary JsObject, we directly retrieve the prototype that
    //    `to_object()` would produce, such as `Number.prototype`, `String.prototype`, etc.
    // Ironswift: borrow the object instead of cloning it (two reference count updates).
    let base;
    let object_ref;
    let object: &JsObject = if let Some(o) = object.as_object_ref() {
        object_ref = o;
        &object_ref
    } else {
        base = object.base_class(context)?;
        &base
    };

    let ic = &context.vm.frame().code_block().ic[usize::from(index)];
    let object_borrowed = object.borrow();
    if let Some(slot) = ic.get(object_borrowed.shape()) {
        let mut result = if slot.attributes.contains(SlotAttributes::PROTOTYPE) {
            let prototype = object_borrowed.shape().prototype().expect("prototype should have value");
            let prototype = prototype.borrow();
            prototype.properties().storage[slot.index as usize].clone()
        } else {
            object_borrowed.properties().storage[slot.index as usize].clone()
        };

        drop(object_borrowed);
        if slot.attributes.has_get() && result.is_object() {
            result =
                result
                    .as_object()
                    .expect("should contain getter")
                    .call(receiver, &[], context)?;
        }
        context.vm.set_register(dst.into(), result);
        return Ok(());
    }

    drop(object_borrowed);

    // Ironswift: property found two or more prototypes up (see `DeepEntry`).
    if let Some((holder, slot)) = deep_hit(ic, object) {
        let mut result = holder.borrow().properties().storage[slot.index as usize].clone();
        if slot.attributes.has_get() && result.is_object() {
            result = result
                .as_object()
                .expect("should contain getter")
                .call(receiver, &[], context)?;
        }
        context.vm.set_register(dst.into(), result);
        return Ok(());
    }

    let key: PropertyKey = ic.name.clone().into();

    let context = &mut InternalMethodPropertyContext::new(context);
    let result = object.__get__(&key, receiver.clone(), context)?;

    // Cache the property.
    let slot = *context.slot();
    if slot.is_cacheable() {
        let ic = &context.vm.frame().code_block.ic[usize::from(index)];
        let object_borrowed = object.borrow();
        let shape = object_borrowed.shape();
        ic.set(shape, slot);
    } else if slot.attributes.contains(SlotAttributes::FOUND | SlotAttributes::PROTOTYPE) {
        let ic = &context.vm.frame().code_block.ic[usize::from(index)];
        if let Some(entry) = build_deep(object, &key) {
            *ic.deep.borrow_mut() = Some(Box::new(entry));
        }
    }

    context.vm.set_register(dst.into(), result);
    Ok(())
}

/// Ironswift: whether named property reads on `obj` follow the ordinary algorithm (no proxy,
/// arguments, string, typed array or module namespace behavior).
fn plain_get(obj: &JsObject) -> bool {
    let v = obj.vtable();
    let o = &ORDINARY_INTERNAL_METHODS;
    std::ptr::fn_addr_eq(v.__get__, o.__get__)
        && std::ptr::fn_addr_eq(v.__get_own_property__, o.__get_own_property__)
        && std::ptr::fn_addr_eq(v.__get_prototype_of__, o.__get_prototype_of__)
}

/// Ironswift: records where `key` was found when it is two or more prototypes up from `object`.
fn build_deep(object: &JsObject, key: &PropertyKey) -> Option<DeepEntry> {
    if !matches!(key, PropertyKey::String(_)) || !plain_get(object) {
        return None;
    }
    let receiver_shape = object.borrow().shape().clone();
    if receiver_shape.lookup(key).is_some() {
        return None;
    }
    let mut chain = Vec::new();
    let mut proto = receiver_shape.prototype();
    while let Some(p) = proto {
        if chain.len() >= 8 || !plain_get(&p) {
            return None;
        }
        let shape = p.borrow().shape().clone();
        chain.push(ChainLink::new(&shape));
        if let Some(slot) = shape.lookup(key) {
            if chain.len() < 2 {
                return None; // the ordinary cache covers the direct prototype
            }
            return Some(DeepEntry {
                receiver: ChainLink::new(&receiver_shape),
                chain,
                slot,
                epoch: shape_epoch(),
            });
        }
        proto = shape.prototype();
    }
    None
}

/// Ironswift: the object holding the property and its slot, if the deep entry still applies.
fn deep_hit(ic: &InlineCache, object: &JsObject) -> Option<(JsObject, Slot)> {
    let deep = ic.deep.borrow();
    let entry = deep.as_ref()?;
    if entry.epoch != shape_epoch() {
        return None;
    }
    let mut proto = {
        let o = object.borrow();
        if !entry.receiver.matches(o.shape()) {
            return None;
        }
        o.shape().prototype()
    };
    let last = entry.chain.len() - 1;
    for (i, link) in entry.chain.iter().enumerate() {
        let p = proto?;
        let next = {
            let pb = p.borrow();
            if !link.matches(pb.shape()) {
                return None;
            }
            pb.shape().prototype()
        };
        if i == last {
            return Some((p, entry.slot));
        }
        proto = next;
    }
    None
}

fn get_by_value<const PUSH_KEY: bool>(
    (dst, key, receiver, object): (
        RegisterOperand,
        RegisterOperand,
        RegisterOperand,
        RegisterOperand,
    ),
    context: &mut Context,
) -> JsResult<()> {
    let key_value = context.vm.get_register(key.into()).clone();
    let base = context.vm.get_register(object.into()).clone();
    let object = base.base_class(context)?;
    let key_value = key_value.to_property_key(context)?;

    // Fast Path
    //
    // NOTE: Since we’re using the prototype returned directly by `base_class()`,
    //       we need to handle string primitives separately due to the
    //       string exotic internal methods.
    match &key_value {
        PropertyKey::Index(index) => {
            if object.is_array() {
                let object_borrowed = object.borrow();
                if let Some(element) = object_borrowed.properties().get_dense_property(index.get())
                {
                    if PUSH_KEY {
                        context.vm.set_register(key.into(), key_value.into());
                    }

                    context.vm.set_register(dst.into(), element);
                    return Ok(());
                }
            } else if let Some(string) = base.as_string() {
                let value = string
                    .code_unit_at(index.get() as usize)
                    .map_or_else(JsValue::undefined, |char| {
                        js_string!([char].as_slice()).into()
                    });

                if PUSH_KEY {
                    context.vm.set_register(key.into(), key_value.into());
                }
                context.vm.set_register(dst.into(), value);
                return Ok(());
            }
        }
        PropertyKey::String(string) if *string == StaticJsStrings::LENGTH => {
            if let Some(string) = base.as_string() {
                let value = string.len().into();

                if PUSH_KEY {
                    context.vm.set_register(key.into(), key_value.into());
                }
                context.vm.set_register(dst.into(), value);
                return Ok(());
            }
        }
        _ => {}
    }

    let receiver = context.vm.get_register(receiver.into());

    // Slow path:
    let result = object.__get__(
        &key_value,
        receiver.clone(),
        &mut InternalMethodPropertyContext::new(context),
    )?;

    if PUSH_KEY {
        context.vm.set_register(key.into(), key_value.into());
    }
    context.vm.set_register(dst.into(), result);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GetLengthProperty;

impl GetLengthProperty {
    #[inline(always)]
    pub(crate) fn operation(
        (dst, object, index): (RegisterOperand, RegisterOperand, IndexOperand),
        context: &mut Context,
    ) -> JsResult<()> {
        let object = context.vm.get_register(object.into()).clone();
        get_by_name::<true>((dst, &object, &object, index), context)
    }
}

impl Operation for GetLengthProperty {
    const NAME: &'static str = "GetLengthProperty";
    const INSTRUCTION: &'static str = "INST - GetLengthProperty";
    const COST: u8 = 4;
}

/// `GetPropertyByName` implements the Opcode Operation for `Opcode::GetPropertyByName`
///
/// Operation:
///  - Get a property by name from an object.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GetPropertyByName;

impl GetPropertyByName {
    #[inline(always)]
    pub(crate) fn operation(
        (dst, object, index): (RegisterOperand, RegisterOperand, IndexOperand),
        context: &mut Context,
    ) -> JsResult<()> {
        let object = context.vm.get_register(object.into()).clone();
        get_by_name::<false>((dst, &object, &object, index), context)
    }
}

impl Operation for GetPropertyByName {
    const NAME: &'static str = "GetPropertyByName";
    const INSTRUCTION: &'static str = "INST - GetPropertyByName";
    const COST: u8 = 4;
}

/// `GetPropertyByNameWithThis` implements the Opcode Operation for `Opcode::GetPropertyByNameWithThis`
///
/// Operation:
///  - Get a property by name from an object with this.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GetPropertyByNameWithThis;

impl GetPropertyByNameWithThis {
    #[inline(always)]
    pub(crate) fn operation(
        (dst, receiver, value, index): (
            RegisterOperand,
            RegisterOperand,
            RegisterOperand,
            IndexOperand,
        ),
        context: &mut Context,
    ) -> JsResult<()> {
        let receiver = context.vm.get_register(receiver.into()).clone();
        let object = context.vm.get_register(value.into()).clone();
        get_by_name::<false>((dst, &object, &receiver, index), context)
    }
}

impl Operation for GetPropertyByNameWithThis {
    const NAME: &'static str = "GetPropertyByNameWithThis";
    const INSTRUCTION: &'static str = "INST - GetPropertyByNameWithThis";
    const COST: u8 = 4;
}

/// `GetPropertyByValue` implements the Opcode Operation for `Opcode::GetPropertyByValue`
///
/// Operation:
///  - Get a property by value from an object and store it in dst.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GetPropertyByValue;

impl GetPropertyByValue {
    #[inline(always)]
    pub(crate) fn operation(
        args: (
            RegisterOperand,
            RegisterOperand,
            RegisterOperand,
            RegisterOperand,
        ),
        context: &mut Context,
    ) -> JsResult<()> {
        get_by_value::<false>(args, context)
    }
}

impl Operation for GetPropertyByValue {
    const NAME: &'static str = "GetPropertyByValue";
    const INSTRUCTION: &'static str = "INST - GetPropertyByValue";
    const COST: u8 = 4;
}

/// `GetPropertyByValuePush` implements the Opcode Operation for `Opcode::GetPropertyByValuePush`
///
/// Operation:
///  - Get a property by value from an object and store the key and value in registers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GetPropertyByValuePush;

impl GetPropertyByValuePush {
    #[inline(always)]
    pub(crate) fn operation(
        args: (
            RegisterOperand,
            RegisterOperand,
            RegisterOperand,
            RegisterOperand,
        ),
        context: &mut Context,
    ) -> JsResult<()> {
        get_by_value::<true>(args, context)
    }
}

impl Operation for GetPropertyByValuePush {
    const NAME: &'static str = "GetPropertyByValuePush";
    const INSTRUCTION: &'static str = "INST - GetPropertyByValuePush";
    const COST: u8 = 4;
}
