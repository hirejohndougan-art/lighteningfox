// WebGL 1 and 2: WebGLRenderingContext, WebGL2RenderingContext, the WebGL object types and
// extensions. State and drawing live in webgl.rs; this file checks arguments, keeps the
// JavaScript objects, and calls the __gl* natives.
(function () {
  const g = globalThis;
  if (typeof g.WebGLRenderingContext === "function") return;

  // ---- constants (WebGL 1, then WebGL 2)
  const K1 = {
    DEPTH_BUFFER_BIT: 0x100, STENCIL_BUFFER_BIT: 0x400, COLOR_BUFFER_BIT: 0x4000, POINTS: 0, LINES: 1, LINE_LOOP: 2, LINE_STRIP: 3,
    TRIANGLES: 4, TRIANGLE_STRIP: 5, TRIANGLE_FAN: 6, ZERO: 0, ONE: 1, SRC_COLOR: 0x300, ONE_MINUS_SRC_COLOR: 0x301, SRC_ALPHA: 0x302,
    ONE_MINUS_SRC_ALPHA: 0x303, DST_ALPHA: 0x304, ONE_MINUS_DST_ALPHA: 0x305, DST_COLOR: 0x306, ONE_MINUS_DST_COLOR: 0x307,
    SRC_ALPHA_SATURATE: 0x308, FUNC_ADD: 0x8006, BLEND_EQUATION: 0x8009, BLEND_EQUATION_RGB: 0x8009, BLEND_EQUATION_ALPHA: 0x883d,
    FUNC_SUBTRACT: 0x800a, FUNC_REVERSE_SUBTRACT: 0x800b, BLEND_DST_RGB: 0x80c8, BLEND_SRC_RGB: 0x80c9, BLEND_DST_ALPHA: 0x80ca,
    BLEND_SRC_ALPHA: 0x80cb, CONSTANT_COLOR: 0x8001, ONE_MINUS_CONSTANT_COLOR: 0x8002, CONSTANT_ALPHA: 0x8003,
    ONE_MINUS_CONSTANT_ALPHA: 0x8004, BLEND_COLOR: 0x8005, ARRAY_BUFFER: 0x8892, ELEMENT_ARRAY_BUFFER: 0x8893,
    ARRAY_BUFFER_BINDING: 0x8894, ELEMENT_ARRAY_BUFFER_BINDING: 0x8895, STREAM_DRAW: 0x88e0, STATIC_DRAW: 0x88e4, DYNAMIC_DRAW: 0x88e8,
    BUFFER_SIZE: 0x8764, BUFFER_USAGE: 0x8765, CURRENT_VERTEX_ATTRIB: 0x8626, FRONT: 0x404, BACK: 0x405, FRONT_AND_BACK: 0x408,
    CULL_FACE: 0xb44, BLEND: 0xbe2, DITHER: 0xbd0, STENCIL_TEST: 0xb90, DEPTH_TEST: 0xb71, SCISSOR_TEST: 0xc11,
    POLYGON_OFFSET_FILL: 0x8037, SAMPLE_ALPHA_TO_COVERAGE: 0x809e, SAMPLE_COVERAGE: 0x80a0, NO_ERROR: 0, INVALID_ENUM: 0x500,
    INVALID_VALUE: 0x501, INVALID_OPERATION: 0x502, OUT_OF_MEMORY: 0x505, CW: 0x900, CCW: 0x901, LINE_WIDTH: 0xb21,
    ALIASED_POINT_SIZE_RANGE: 0x846d, ALIASED_LINE_WIDTH_RANGE: 0x846e, CULL_FACE_MODE: 0xb45, FRONT_FACE: 0xb46, DEPTH_RANGE: 0xb70,
    DEPTH_WRITEMASK: 0xb72, DEPTH_CLEAR_VALUE: 0xb73, DEPTH_FUNC: 0xb74, STENCIL_CLEAR_VALUE: 0xb91, STENCIL_FUNC: 0xb92,
    STENCIL_FAIL: 0xb94, STENCIL_PASS_DEPTH_FAIL: 0xb95, STENCIL_PASS_DEPTH_PASS: 0xb96, STENCIL_REF: 0xb97,
    STENCIL_VALUE_MASK: 0xb93, STENCIL_WRITEMASK: 0xb98, STENCIL_BACK_FUNC: 0x8800, STENCIL_BACK_FAIL: 0x8801,
    STENCIL_BACK_PASS_DEPTH_FAIL: 0x8802, STENCIL_BACK_PASS_DEPTH_PASS: 0x8803, STENCIL_BACK_REF: 0x8ca3,
    STENCIL_BACK_VALUE_MASK: 0x8ca4, STENCIL_BACK_WRITEMASK: 0x8ca5, VIEWPORT: 0xba2, SCISSOR_BOX: 0xc10, COLOR_CLEAR_VALUE: 0xc22,
    COLOR_WRITEMASK: 0xc23, UNPACK_ALIGNMENT: 0xcf5, PACK_ALIGNMENT: 0xd05, MAX_TEXTURE_SIZE: 0xd33, MAX_VIEWPORT_DIMS: 0xd3a,
    SUBPIXEL_BITS: 0xd50, RED_BITS: 0xd52, GREEN_BITS: 0xd53, BLUE_BITS: 0xd54, ALPHA_BITS: 0xd55, DEPTH_BITS: 0xd56,
    STENCIL_BITS: 0xd57, POLYGON_OFFSET_UNITS: 0x2a00, POLYGON_OFFSET_FACTOR: 0x8038, TEXTURE_BINDING_2D: 0x8069,
    SAMPLE_BUFFERS: 0x80a8, SAMPLES: 0x80a9, SAMPLE_COVERAGE_VALUE: 0x80aa, SAMPLE_COVERAGE_INVERT: 0x80ab,
    COMPRESSED_TEXTURE_FORMATS: 0x86a3, DONT_CARE: 0x1100, FASTEST: 0x1101, NICEST: 0x1102, GENERATE_MIPMAP_HINT: 0x8192,
    BYTE: 0x1400, UNSIGNED_BYTE: 0x1401, SHORT: 0x1402, UNSIGNED_SHORT: 0x1403, INT: 0x1404, UNSIGNED_INT: 0x1405, FLOAT: 0x1406,
    DEPTH_COMPONENT: 0x1902, ALPHA: 0x1906, RGB: 0x1907, RGBA: 0x1908, LUMINANCE: 0x1909, LUMINANCE_ALPHA: 0x190a,
    UNSIGNED_SHORT_4_4_4_4: 0x8033, UNSIGNED_SHORT_5_5_5_1: 0x8034, UNSIGNED_SHORT_5_6_5: 0x8363, FRAGMENT_SHADER: 0x8b30,
    VERTEX_SHADER: 0x8b31, MAX_VERTEX_ATTRIBS: 0x8869, MAX_VERTEX_UNIFORM_VECTORS: 0x8dfb, MAX_VARYING_VECTORS: 0x8dfc,
    MAX_COMBINED_TEXTURE_IMAGE_UNITS: 0x8b4d, MAX_VERTEX_TEXTURE_IMAGE_UNITS: 0x8b4c, MAX_TEXTURE_IMAGE_UNITS: 0x8872,
    MAX_FRAGMENT_UNIFORM_VECTORS: 0x8dfd, SHADER_TYPE: 0x8b4f, DELETE_STATUS: 0x8b80, LINK_STATUS: 0x8b82, VALIDATE_STATUS: 0x8b83,
    ATTACHED_SHADERS: 0x8b85, ACTIVE_UNIFORMS: 0x8b86, ACTIVE_ATTRIBUTES: 0x8b89, SHADING_LANGUAGE_VERSION: 0x8b8c,
    CURRENT_PROGRAM: 0x8b8d, NEVER: 0x200, LESS: 0x201, EQUAL: 0x202, LEQUAL: 0x203, GREATER: 0x204, NOTEQUAL: 0x205, GEQUAL: 0x206,
    ALWAYS: 0x207, KEEP: 0x1e00, REPLACE: 0x1e01, INCR: 0x1e02, DECR: 0x1e03, INVERT: 0x150a, INCR_WRAP: 0x8507, DECR_WRAP: 0x8508,
    VENDOR: 0x1f00, RENDERER: 0x1f01, VERSION: 0x1f02, NEAREST: 0x2600, LINEAR: 0x2601, NEAREST_MIPMAP_NEAREST: 0x2700,
    LINEAR_MIPMAP_NEAREST: 0x2701, NEAREST_MIPMAP_LINEAR: 0x2702, LINEAR_MIPMAP_LINEAR: 0x2703, TEXTURE_MAG_FILTER: 0x2800,
    TEXTURE_MIN_FILTER: 0x2801, TEXTURE_WRAP_S: 0x2802, TEXTURE_WRAP_T: 0x2803, TEXTURE_2D: 0xde1, TEXTURE: 0x1702,
    TEXTURE_CUBE_MAP: 0x8513, TEXTURE_BINDING_CUBE_MAP: 0x8514, TEXTURE_CUBE_MAP_POSITIVE_X: 0x8515,
    TEXTURE_CUBE_MAP_NEGATIVE_X: 0x8516, TEXTURE_CUBE_MAP_POSITIVE_Y: 0x8517, TEXTURE_CUBE_MAP_NEGATIVE_Y: 0x8518,
    TEXTURE_CUBE_MAP_POSITIVE_Z: 0x8519, TEXTURE_CUBE_MAP_NEGATIVE_Z: 0x851a, MAX_CUBE_MAP_TEXTURE_SIZE: 0x851c,
    ACTIVE_TEXTURE: 0x84e0, REPEAT: 0x2901, CLAMP_TO_EDGE: 0x812f, MIRRORED_REPEAT: 0x8370, FLOAT_VEC2: 0x8b50, FLOAT_VEC3: 0x8b51,
    FLOAT_VEC4: 0x8b52, INT_VEC2: 0x8b53, INT_VEC3: 0x8b54, INT_VEC4: 0x8b55, BOOL: 0x8b56, BOOL_VEC2: 0x8b57, BOOL_VEC3: 0x8b58,
    BOOL_VEC4: 0x8b59, FLOAT_MAT2: 0x8b5a, FLOAT_MAT3: 0x8b5b, FLOAT_MAT4: 0x8b5c, SAMPLER_2D: 0x8b5e, SAMPLER_CUBE: 0x8b60,
    VERTEX_ATTRIB_ARRAY_ENABLED: 0x8622, VERTEX_ATTRIB_ARRAY_SIZE: 0x8623, VERTEX_ATTRIB_ARRAY_STRIDE: 0x8624,
    VERTEX_ATTRIB_ARRAY_TYPE: 0x8625, VERTEX_ATTRIB_ARRAY_NORMALIZED: 0x886a, VERTEX_ATTRIB_ARRAY_POINTER: 0x8645,
    VERTEX_ATTRIB_ARRAY_BUFFER_BINDING: 0x889f, IMPLEMENTATION_COLOR_READ_TYPE: 0x8b9a, IMPLEMENTATION_COLOR_READ_FORMAT: 0x8b9b,
    COMPILE_STATUS: 0x8b81, LOW_FLOAT: 0x8df0, MEDIUM_FLOAT: 0x8df1, HIGH_FLOAT: 0x8df2, LOW_INT: 0x8df3, MEDIUM_INT: 0x8df4,
    HIGH_INT: 0x8df5, FRAMEBUFFER: 0x8d40, RENDERBUFFER: 0x8d41, RGBA4: 0x8056, RGB5_A1: 0x8057, RGB565: 0x8d62,
    DEPTH_COMPONENT16: 0x81a5, STENCIL_INDEX8: 0x8d48, DEPTH_STENCIL: 0x84f9, RENDERBUFFER_WIDTH: 0x8d42,
    RENDERBUFFER_HEIGHT: 0x8d43, RENDERBUFFER_INTERNAL_FORMAT: 0x8d44, RENDERBUFFER_RED_SIZE: 0x8d50,
    RENDERBUFFER_GREEN_SIZE: 0x8d51, RENDERBUFFER_BLUE_SIZE: 0x8d52, RENDERBUFFER_ALPHA_SIZE: 0x8d53,
    RENDERBUFFER_DEPTH_SIZE: 0x8d54, RENDERBUFFER_STENCIL_SIZE: 0x8d55, FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE: 0x8cd0,
    FRAMEBUFFER_ATTACHMENT_OBJECT_NAME: 0x8cd1, FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL: 0x8cd2,
    FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE: 0x8cd3, COLOR_ATTACHMENT0: 0x8ce0, DEPTH_ATTACHMENT: 0x8d00,
    STENCIL_ATTACHMENT: 0x8d20, DEPTH_STENCIL_ATTACHMENT: 0x821a, NONE: 0, FRAMEBUFFER_COMPLETE: 0x8cd5,
    FRAMEBUFFER_INCOMPLETE_ATTACHMENT: 0x8cd6, FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT: 0x8cd7,
    FRAMEBUFFER_INCOMPLETE_DIMENSIONS: 0x8cd9, FRAMEBUFFER_UNSUPPORTED: 0x8cdd, FRAMEBUFFER_BINDING: 0x8ca6,
    RENDERBUFFER_BINDING: 0x8ca7, MAX_RENDERBUFFER_SIZE: 0x84e8, INVALID_FRAMEBUFFER_OPERATION: 0x506, UNPACK_FLIP_Y_WEBGL: 0x9240,
    UNPACK_PREMULTIPLY_ALPHA_WEBGL: 0x9241, CONTEXT_LOST_WEBGL: 0x9242, UNPACK_COLORSPACE_CONVERSION_WEBGL: 0x9243,
    BROWSER_DEFAULT_WEBGL: 0x9244,
  };
  const K2 = {
    READ_BUFFER: 0xc02, UNPACK_ROW_LENGTH: 0xcf2, UNPACK_SKIP_ROWS: 0xcf3, UNPACK_SKIP_PIXELS: 0xcf4, PACK_ROW_LENGTH: 0xd02,
    PACK_SKIP_ROWS: 0xd03, PACK_SKIP_PIXELS: 0xd04, COLOR: 0x1800, DEPTH: 0x1801, STENCIL: 0x1802, RED: 0x1903, RGB8: 0x8051,
    RGBA8: 0x8058, RGB10_A2: 0x8059, TEXTURE_BINDING_3D: 0x806a, UNPACK_SKIP_IMAGES: 0x806d, UNPACK_IMAGE_HEIGHT: 0x806e,
    TEXTURE_3D: 0x806f, TEXTURE_WRAP_R: 0x8072, MAX_3D_TEXTURE_SIZE: 0x8073, UNSIGNED_INT_2_10_10_10_REV: 0x8368,
    MAX_ELEMENTS_VERTICES: 0x80e8, MAX_ELEMENTS_INDICES: 0x80e9, TEXTURE_MIN_LOD: 0x813a, TEXTURE_MAX_LOD: 0x813b,
    TEXTURE_BASE_LEVEL: 0x813c, TEXTURE_MAX_LEVEL: 0x813d, MIN: 0x8007, MAX: 0x8008, DEPTH_COMPONENT24: 0x81a6,
    MAX_TEXTURE_LOD_BIAS: 0x84fd, TEXTURE_COMPARE_MODE: 0x884c, TEXTURE_COMPARE_FUNC: 0x884d, CURRENT_QUERY: 0x8865,
    QUERY_RESULT: 0x8866, QUERY_RESULT_AVAILABLE: 0x8867, STREAM_READ: 0x88e1, STREAM_COPY: 0x88e2, STATIC_READ: 0x88e5,
    STATIC_COPY: 0x88e6, DYNAMIC_READ: 0x88e9, DYNAMIC_COPY: 0x88ea, MAX_DRAW_BUFFERS: 0x8824, DRAW_BUFFER0: 0x8825,
    DRAW_BUFFER1: 0x8826, DRAW_BUFFER2: 0x8827, DRAW_BUFFER3: 0x8828, DRAW_BUFFER4: 0x8829, DRAW_BUFFER5: 0x882a,
    DRAW_BUFFER6: 0x882b, DRAW_BUFFER7: 0x882c, DRAW_BUFFER8: 0x882d, DRAW_BUFFER9: 0x882e, DRAW_BUFFER10: 0x882f,
    DRAW_BUFFER11: 0x8830, DRAW_BUFFER12: 0x8831, DRAW_BUFFER13: 0x8832, DRAW_BUFFER14: 0x8833, DRAW_BUFFER15: 0x8834,
    MAX_FRAGMENT_UNIFORM_COMPONENTS: 0x8b49, MAX_VERTEX_UNIFORM_COMPONENTS: 0x8b4a, SAMPLER_3D: 0x8b5f, SAMPLER_2D_SHADOW: 0x8b62,
    FRAGMENT_SHADER_DERIVATIVE_HINT: 0x8b8b, PIXEL_PACK_BUFFER: 0x88eb, PIXEL_UNPACK_BUFFER: 0x88ec,
    PIXEL_PACK_BUFFER_BINDING: 0x88ed, PIXEL_UNPACK_BUFFER_BINDING: 0x88ef, FLOAT_MAT2x3: 0x8b65, FLOAT_MAT2x4: 0x8b66,
    FLOAT_MAT3x2: 0x8b67, FLOAT_MAT3x4: 0x8b68, FLOAT_MAT4x2: 0x8b69, FLOAT_MAT4x3: 0x8b6a, SRGB: 0x8c40, SRGB8: 0x8c41,
    SRGB8_ALPHA8: 0x8c43, COMPARE_REF_TO_TEXTURE: 0x884e, RGBA32F: 0x8814, RGB32F: 0x8815, RGBA16F: 0x881a, RGB16F: 0x881b,
    VERTEX_ATTRIB_ARRAY_INTEGER: 0x88fd, MAX_ARRAY_TEXTURE_LAYERS: 0x88ff, MIN_PROGRAM_TEXEL_OFFSET: 0x8904,
    MAX_PROGRAM_TEXEL_OFFSET: 0x8905, MAX_VARYING_COMPONENTS: 0x8b4b, TEXTURE_2D_ARRAY: 0x8c1a, TEXTURE_BINDING_2D_ARRAY: 0x8c1d,
    R11F_G11F_B10F: 0x8c3a, UNSIGNED_INT_10F_11F_11F_REV: 0x8c3b, RGB9_E5: 0x8c3d, UNSIGNED_INT_5_9_9_9_REV: 0x8c3e,
    TRANSFORM_FEEDBACK_BUFFER_MODE: 0x8c7f, MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS: 0x8c80,
    TRANSFORM_FEEDBACK_VARYINGS: 0x8c83, TRANSFORM_FEEDBACK_BUFFER_START: 0x8c84, TRANSFORM_FEEDBACK_BUFFER_SIZE: 0x8c85,
    TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN: 0x8c88, RASTERIZER_DISCARD: 0x8c89,
    MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS: 0x8c8a, MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS: 0x8c8b,
    INTERLEAVED_ATTRIBS: 0x8c8c, SEPARATE_ATTRIBS: 0x8c8d, TRANSFORM_FEEDBACK_BUFFER: 0x8c8e,
    TRANSFORM_FEEDBACK_BUFFER_BINDING: 0x8c8f, RGBA32UI: 0x8d70, RGB32UI: 0x8d71, RGBA16UI: 0x8d76, RGB16UI: 0x8d77,
    RGBA8UI: 0x8d7c, RGB8UI: 0x8d7d, RGBA32I: 0x8d82, RGB32I: 0x8d83, RGBA16I: 0x8d88, RGB16I: 0x8d89, RGBA8I: 0x8d8e,
    RGB8I: 0x8d8f, RED_INTEGER: 0x8d94, RGB_INTEGER: 0x8d98, RGBA_INTEGER: 0x8d99, SAMPLER_2D_ARRAY: 0x8dc1,
    SAMPLER_2D_ARRAY_SHADOW: 0x8dc4, SAMPLER_CUBE_SHADOW: 0x8dc5, UNSIGNED_INT_VEC2: 0x8dc6, UNSIGNED_INT_VEC3: 0x8dc7,
    UNSIGNED_INT_VEC4: 0x8dc8, INT_SAMPLER_2D: 0x8dca, INT_SAMPLER_3D: 0x8dcb, INT_SAMPLER_CUBE: 0x8dcc,
    INT_SAMPLER_2D_ARRAY: 0x8dcf, UNSIGNED_INT_SAMPLER_2D: 0x8dd2, UNSIGNED_INT_SAMPLER_3D: 0x8dd3,
    UNSIGNED_INT_SAMPLER_CUBE: 0x8dd4, UNSIGNED_INT_SAMPLER_2D_ARRAY: 0x8dd7, DEPTH_COMPONENT32F: 0x8cac,
    DEPTH32F_STENCIL8: 0x8cad, FLOAT_32_UNSIGNED_INT_24_8_REV: 0x8dad, FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING: 0x8210,
    FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE: 0x8211, FRAMEBUFFER_ATTACHMENT_RED_SIZE: 0x8212,
    FRAMEBUFFER_ATTACHMENT_GREEN_SIZE: 0x8213, FRAMEBUFFER_ATTACHMENT_BLUE_SIZE: 0x8214, FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE: 0x8215,
    FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE: 0x8216, FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE: 0x8217, FRAMEBUFFER_DEFAULT: 0x8218,
    UNSIGNED_INT_24_8: 0x84fa, DEPTH24_STENCIL8: 0x88f0, UNSIGNED_NORMALIZED: 0x8c17, DRAW_FRAMEBUFFER_BINDING: 0x8ca6,
    READ_FRAMEBUFFER: 0x8ca8, DRAW_FRAMEBUFFER: 0x8ca9, READ_FRAMEBUFFER_BINDING: 0x8caa, RENDERBUFFER_SAMPLES: 0x8cab,
    FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER: 0x8cd4, MAX_COLOR_ATTACHMENTS: 0x8cdf, COLOR_ATTACHMENT1: 0x8ce1,
    COLOR_ATTACHMENT2: 0x8ce2, COLOR_ATTACHMENT3: 0x8ce3, COLOR_ATTACHMENT4: 0x8ce4, COLOR_ATTACHMENT5: 0x8ce5,
    COLOR_ATTACHMENT6: 0x8ce6, COLOR_ATTACHMENT7: 0x8ce7, COLOR_ATTACHMENT8: 0x8ce8, COLOR_ATTACHMENT9: 0x8ce9,
    COLOR_ATTACHMENT10: 0x8cea, COLOR_ATTACHMENT11: 0x8ceb, COLOR_ATTACHMENT12: 0x8cec, COLOR_ATTACHMENT13: 0x8ced,
    COLOR_ATTACHMENT14: 0x8cee, COLOR_ATTACHMENT15: 0x8cef, FRAMEBUFFER_INCOMPLETE_MULTISAMPLE: 0x8d56, MAX_SAMPLES: 0x8d57,
    HALF_FLOAT: 0x140b, RG: 0x8227, RG_INTEGER: 0x8228, R8: 0x8229, RG8: 0x822b, R16F: 0x822d, R32F: 0x822e, RG16F: 0x822f,
    RG32F: 0x8230, R8I: 0x8231, R8UI: 0x8232, R16I: 0x8233, R16UI: 0x8234, R32I: 0x8235, R32UI: 0x8236, RG8I: 0x8237,
    RG8UI: 0x8238, RG16I: 0x8239, RG16UI: 0x823a, RG32I: 0x823b, RG32UI: 0x823c, VERTEX_ARRAY_BINDING: 0x85b5,
    R8_SNORM: 0x8f94, RG8_SNORM: 0x8f95, RGB8_SNORM: 0x8f96, RGBA8_SNORM: 0x8f97, SIGNED_NORMALIZED: 0x8f9c,
    COPY_READ_BUFFER: 0x8f36, COPY_WRITE_BUFFER: 0x8f37, COPY_READ_BUFFER_BINDING: 0x8f36, COPY_WRITE_BUFFER_BINDING: 0x8f37,
    UNIFORM_BUFFER: 0x8a11, UNIFORM_BUFFER_BINDING: 0x8a28, UNIFORM_BUFFER_START: 0x8a29, UNIFORM_BUFFER_SIZE: 0x8a2a,
    MAX_VERTEX_UNIFORM_BLOCKS: 0x8a2b, MAX_FRAGMENT_UNIFORM_BLOCKS: 0x8a2d, MAX_COMBINED_UNIFORM_BLOCKS: 0x8a2e,
    MAX_UNIFORM_BUFFER_BINDINGS: 0x8a2f, MAX_UNIFORM_BLOCK_SIZE: 0x8a30, MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS: 0x8a31,
    MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS: 0x8a33, UNIFORM_BUFFER_OFFSET_ALIGNMENT: 0x8a34, ACTIVE_UNIFORM_BLOCKS: 0x8a36,
    UNIFORM_TYPE: 0x8a37, UNIFORM_SIZE: 0x8a38, UNIFORM_BLOCK_INDEX: 0x8a3a, UNIFORM_OFFSET: 0x8a3b, UNIFORM_ARRAY_STRIDE: 0x8a3c,
    UNIFORM_MATRIX_STRIDE: 0x8a3d, UNIFORM_IS_ROW_MAJOR: 0x8a3e, UNIFORM_BLOCK_BINDING: 0x8a3f, UNIFORM_BLOCK_DATA_SIZE: 0x8a40,
    UNIFORM_BLOCK_ACTIVE_UNIFORMS: 0x8a42, UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES: 0x8a43,
    UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER: 0x8a44, UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER: 0x8a46,
    INVALID_INDEX: 0xffffffff, MAX_VERTEX_OUTPUT_COMPONENTS: 0x9122, MAX_FRAGMENT_INPUT_COMPONENTS: 0x9125,
    MAX_SERVER_WAIT_TIMEOUT: 0x9111, OBJECT_TYPE: 0x9112, SYNC_CONDITION: 0x9113, SYNC_STATUS: 0x9114, SYNC_FLAGS: 0x9115,
    SYNC_FENCE: 0x9116, SYNC_GPU_COMMANDS_COMPLETE: 0x9117, UNSIGNALED: 0x9118, SIGNALED: 0x9119, ALREADY_SIGNALED: 0x911a,
    TIMEOUT_EXPIRED: 0x911b, CONDITION_SATISFIED: 0x911c, WAIT_FAILED: 0x911d, SYNC_FLUSH_COMMANDS_BIT: 1,
    VERTEX_ATTRIB_ARRAY_DIVISOR: 0x88fe, ANY_SAMPLES_PASSED: 0x8c2f, ANY_SAMPLES_PASSED_CONSERVATIVE: 0x8d6a,
    SAMPLER_BINDING: 0x8919, RGB10_A2UI: 0x906f, INT_2_10_10_10_REV: 0x8d9f, TRANSFORM_FEEDBACK: 0x8e22,
    TRANSFORM_FEEDBACK_PAUSED: 0x8e23, TRANSFORM_FEEDBACK_ACTIVE: 0x8e24, TRANSFORM_FEEDBACK_BINDING: 0x8e25,
    TEXTURE_IMMUTABLE_FORMAT: 0x912f, MAX_ELEMENT_INDEX: 0x8d6b, TEXTURE_IMMUTABLE_LEVELS: 0x82df, TIMEOUT_IGNORED: -1,
    MAX_CLIENT_WAIT_TIMEOUT_WEBGL: 0x9247,
  };

  for (let i = 0; i < 32; i++) K1["TEXTURE" + i] = 0x84c0 + i;

  // ---- object types
  const kinds = { buffer: 0, texture: 1, framebuffer: 2, renderbuffer: 3, shader: 4, program: 5, vao: 6, sampler: 7, query: 8, sync: 9 };
  class WebGLObject {}
  const make = (name) => {
    const C = class extends WebGLObject {};
    Object.defineProperty(C, "name", { value: name });
    g[name] = C;
    return C;
  };
  const WebGLBuffer = make("WebGLBuffer"), WebGLTexture = make("WebGLTexture"), WebGLFramebuffer = make("WebGLFramebuffer");
  const WebGLRenderbuffer = make("WebGLRenderbuffer"), WebGLShader = make("WebGLShader"), WebGLProgram = make("WebGLProgram");
  const WebGLVertexArrayObject = make("WebGLVertexArrayObject"), WebGLSampler = make("WebGLSampler"), WebGLQuery = make("WebGLQuery");
  const WebGLSync = make("WebGLSync"), WebGLTransformFeedback = make("WebGLTransformFeedback");
  const WebGLVertexArrayObjectOES = make("WebGLVertexArrayObjectOES");
  const classOf = [WebGLBuffer, WebGLTexture, WebGLFramebuffer, WebGLRenderbuffer, WebGLShader, WebGLProgram, WebGLVertexArrayObject, WebGLSampler, WebGLQuery, WebGLSync];
  class WebGLUniformLocation {}
  class WebGLActiveInfo {
    constructor(size, type, name) { this.size = size; this.type = type; this.name = name; }
  }
  class WebGLShaderPrecisionFormat {
    constructor(min, max, p) { this.rangeMin = min; this.rangeMax = max; this.precision = p; }
  }
  class WebGLContextEvent extends (g.Event || Object) {
    constructor(type, init) { super(type, init); this.statusMessage = (init && init.statusMessage) || ""; }
  }
  Object.assign(g, { WebGLObject, WebGLUniformLocation, WebGLActiveInfo, WebGLShaderPrecisionFormat, WebGLContextEvent });

  // ---- the context
  const ctxs = new Map();
  const OBJ_PARAMS = new Set([0x8894, 0x8895, 0x8b8d, 0x8ca6, 0x8caa, 0x8ca7, 0x8069, 0x806a, 0x8514, 0x8c1d, 0x85b5, 0x8a28, 0x8f36, 0x8f37, 0x88ed, 0x88ef, 0x8c8f, 0x8919, 0x8e25]);
  const typedOf = (base) => (base === 0 ? Float32Array : base === 1 ? Int32Array : Uint32Array);
  const isView = (v) => ArrayBuffer.isView(v);

  function Base() {}
  const P = Base.prototype;

  P.__init = function (canvas, attrs, v2) {
    this.__canvas = canvas;
    this.__cid = canvas.__id;
    this.__v2 = v2;
    this.__objs = new Map();
    this.__lost = false;
    this.__exts = new Map();
    attrs = attrs || {};
    const pick = (k, d) => (attrs[k] === undefined ? d : !!attrs[k]);
    this.__attrs = {
      alpha: pick("alpha", true), depth: pick("depth", true), stencil: pick("stencil", false), antialias: pick("antialias", true),
      premultipliedAlpha: pick("premultipliedAlpha", true), preserveDrawingBuffer: pick("preserveDrawingBuffer", false),
      powerPreference: attrs.powerPreference || "default", failIfMajorPerformanceCaveat: pick("failIfMajorPerformanceCaveat", false),
      desynchronized: pick("desynchronized", false), xrCompatible: false,
    };
    const a = this.__attrs;
    __glCreate(this.__cid, this.__w(), this.__h(), [a.alpha, a.depth, a.stencil, a.antialias, a.premultipliedAlpha, a.preserveDrawingBuffer], v2);
  };
  P.__w = function () { return this.__canvas.__offscreenSize ? this.__canvas.__offscreenSize[0] : +this.__canvas.width || 0; };
  P.__h = function () { return this.__canvas.__offscreenSize ? this.__canvas.__offscreenSize[1] : +this.__canvas.height || 0; };
  P.__err = function (code) { __glSetError(this.__cid, code); };
  P.__new = function (kind, extra) {
    const id = __glGen(this.__cid, kind, extra || 0);
    if (!id) return null;
    const o = new classOf[kind]();
    Object.defineProperty(o, "__id", { value: id });
    Object.defineProperty(o, "__ctx", { value: this });
    o.__deleted = false;
    this.__objs.set(id, o);
    return o;
  };
  // An object argument's id: 0 for null, -1 (after reporting an error) if it's wrong.
  P.__id = function (o, cls, allowDeleted) {
    if (o === null || o === undefined) return 0;
    if (!(o instanceof cls)) throw new TypeError("Argument is not a " + cls.name);
    if (o.__ctx !== this) { this.__err(0x502); return -1; }
    if (o.__deleted && !allowDeleted) { this.__err(0x502); return -1; }
    return o.__id;
  };
  P.__obj = function (id) { return id ? this.__objs.get(id) || null : null; };
  P.__del = function (o, cls) {
    if (o === null || o === undefined) return;
    const id = this.__id(o, cls, true);
    if (id <= 0 || o.__deleted) return;
    o.__deleted = true;
    __glDel(this.__cid, 0, id);
  };
  P.__is = function (o, cls, kind) {
    if (!(o instanceof cls) || o.__ctx !== this || o.__deleted) return false;
    return __glIs(this.__cid, kind, o.__id);
  };

  Object.defineProperty(P, "canvas", { get() { return this.__canvas.__offscreenOwner || this.__canvas; } });
  Object.defineProperty(P, "drawingBufferWidth", { get() { return this.__w(); } });
  Object.defineProperty(P, "drawingBufferHeight", { get() { return this.__h(); } });
  Object.defineProperty(P, "drawingBufferColorSpace", { get() { return "srgb"; }, set(_) {} });
  Object.defineProperty(P, "unpackColorSpace", { get() { return "srgb"; }, set(_) {} });

  P.getContextAttributes = function () { return this.__lost ? null : Object.assign({}, this.__attrs, { antialias: false }); };
  P.isContextLost = function () { return this.__lost; };
  // HDR output tone mapping: only "standard" output exists here, but the setting is kept.
  P.drawingBufferToneMapping = function (opts) {
    if (opts !== undefined && opts !== null) {
      if (typeof opts !== "object") throw new TypeError("drawingBufferToneMapping: expected a dictionary");
      const m = opts.mode === undefined ? "standard" : String(opts.mode);
      if (m !== "standard" && m !== "extended") throw new TypeError("drawingBufferToneMapping: bad mode '" + m + "'");
      this.__toneMapping = m;
    }
    return { mode: this.__toneMapping || "standard" };
  };
  P.getError = function () { return __glError(this.__cid); };
  P.finish = function () {};
  P.flush = function () {};
  P.makeXRCompatible = function () { return Promise.resolve(); };

  // ---- extensions
  const EXT1 = ["ANGLE_instanced_arrays", "EXT_blend_minmax", "EXT_color_buffer_half_float", "EXT_float_blend", "EXT_frag_depth",
    "EXT_shader_texture_lod", "EXT_sRGB", "EXT_texture_filter_anisotropic", "OES_element_index_uint", "OES_fbo_render_mipmap",
    "OES_standard_derivatives", "OES_texture_float", "OES_texture_float_linear", "OES_texture_half_float",
    "OES_texture_half_float_linear", "OES_vertex_array_object", "WEBGL_color_buffer_float", "WEBGL_depth_texture",
    "WEBGL_draw_buffers", "WEBGL_lose_context", "KHR_parallel_shader_compile", "WEBGL_compressed_texture_s3tc",
    "WEBGL_compressed_texture_s3tc_srgb"];
  const EXT2 = ["EXT_color_buffer_float", "EXT_color_buffer_half_float", "EXT_float_blend", "EXT_texture_filter_anisotropic",
    "OES_texture_float_linear", "WEBGL_lose_context", "KHR_parallel_shader_compile", "OES_draw_buffers_indexed",
    "WEBGL_compressed_texture_s3tc", "WEBGL_compressed_texture_s3tc_srgb"];
  P.getSupportedExtensions = function () { return (this.__v2 ? EXT2 : EXT1).slice(); };
  P.getExtension = function (name) {
    name = String(name);
    const list = this.__v2 ? EXT2 : EXT1;
    const real = list.find((n) => n.toLowerCase() === name.toLowerCase());
    if (!real) return null;
    if (this.__exts.has(real)) return this.__exts.get(real);
    const gl = this;
    let x = {};
    switch (real) {
      case "ANGLE_instanced_arrays":
        x = {
          VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE: 0x88fe,
          drawArraysInstancedANGLE: (m, f, c, n) => gl.__drawArrays(m, f, c, n),
          drawElementsInstancedANGLE: (m, c, t, o, n) => gl.__drawElements(m, c, t, o, n),
          vertexAttribDivisorANGLE: (i, d) => __glVADivisor(gl.__cid, i, d),
        };
        break;
      case "OES_vertex_array_object":
        x = {
          VERTEX_ARRAY_BINDING_OES: 0x85b5,
          createVertexArrayOES: () => gl.__new(6),
          deleteVertexArrayOES: (v) => gl.deleteVertexArray(v),
          isVertexArrayOES: (v) => gl.isVertexArray(v),
          bindVertexArrayOES: (v) => gl.bindVertexArray(v),
        };
        break;
      case "OES_standard_derivatives": x = { FRAGMENT_SHADER_DERIVATIVE_HINT_OES: 0x8b8b }; break;
      case "OES_texture_half_float": x = { HALF_FLOAT_OES: 0x8d61 }; break;
      case "EXT_blend_minmax": x = { MIN_EXT: 0x8007, MAX_EXT: 0x8008 }; break;
      case "WEBGL_depth_texture": x = { UNSIGNED_INT_24_8_WEBGL: 0x84fa }; break;
      case "EXT_sRGB": x = { SRGB_EXT: 0x8c40, SRGB_ALPHA_EXT: 0x8c42, SRGB8_ALPHA8_EXT: 0x8c43, FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING_EXT: 0x8210 }; break;
      case "EXT_texture_filter_anisotropic": x = { TEXTURE_MAX_ANISOTROPY_EXT: 0x84fe, MAX_TEXTURE_MAX_ANISOTROPY_EXT: 0x84ff }; break;
      case "WEBGL_color_buffer_float": x = { RGBA32F_EXT: 0x8814, FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT: 0x8211, UNSIGNED_NORMALIZED_EXT: 0x8c17 }; break;
      case "EXT_color_buffer_half_float": x = { RGBA16F_EXT: 0x881a, RGB16F_EXT: 0x881b, FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT: 0x8211, UNSIGNED_NORMALIZED_EXT: 0x8c17 }; break;
      case "KHR_parallel_shader_compile": x = { COMPLETION_STATUS_KHR: 0x91b1 }; break;
      case "WEBGL_compressed_texture_s3tc":
        x = { COMPRESSED_RGB_S3TC_DXT1_EXT: 0x83f0, COMPRESSED_RGBA_S3TC_DXT1_EXT: 0x83f1, COMPRESSED_RGBA_S3TC_DXT3_EXT: 0x83f2, COMPRESSED_RGBA_S3TC_DXT5_EXT: 0x83f3 };
        __glEnableExt(this.__cid, real);
        this.__compressed = [0x83f0, 0x83f1, 0x83f2, 0x83f3].concat(this.__compressed || []);
        break;
      case "WEBGL_compressed_texture_s3tc_srgb":
        x = { COMPRESSED_SRGB_S3TC_DXT1_EXT: 0x8c4c, COMPRESSED_SRGB_ALPHA_S3TC_DXT1_EXT: 0x8c4d, COMPRESSED_SRGB_ALPHA_S3TC_DXT3_EXT: 0x8c4e, COMPRESSED_SRGB_ALPHA_S3TC_DXT5_EXT: 0x8c4f };
        __glEnableExt(this.__cid, real);
        this.__compressed = (this.__compressed || []).concat([0x8c4c, 0x8c4d, 0x8c4e, 0x8c4f]);
        break;
      case "WEBGL_lose_context":
        x = {
          loseContext: () => {
            if (gl.__lost) return gl.__err(0x502);
            gl.__lost = true;
            setTimeout(() => gl.canvas.dispatchEvent && gl.canvas.dispatchEvent(new WebGLContextEvent("webglcontextlost", { cancelable: true })), 0);
          },
          restoreContext: () => {
            if (!gl.__lost) return gl.__err(0x502);
            gl.__lost = false;
            setTimeout(() => gl.canvas.dispatchEvent && gl.canvas.dispatchEvent(new WebGLContextEvent("webglcontextrestored")), 0);
          },
        };
        break;
      case "WEBGL_draw_buffers":
        x = { drawBuffersWEBGL: (b) => __glDrawBuffers(gl.__cid, Array.from(b)), MAX_COLOR_ATTACHMENTS_WEBGL: 0x8cdf, MAX_DRAW_BUFFERS_WEBGL: 0x8824 };
        for (let i = 0; i < 16; i++) { x["COLOR_ATTACHMENT" + i + "_WEBGL"] = 0x8ce0 + i; x["DRAW_BUFFER" + i + "_WEBGL"] = 0x8825 + i; }
        break;
      case "OES_draw_buffers_indexed":
        x = { enableiOES() {}, disableiOES() {}, blendEquationiOES() {}, blendEquationSeparateiOES() {}, blendFunciOES() {}, blendFuncSeparateiOES() {}, colorMaskiOES() {} };
        break;
      default: x = {};
    }
    this.__exts.set(real, x);
    return x;
  };

  // ---- buffers
  P.createBuffer = function () { return this.__new(0); };
  P.deleteBuffer = function (b) { this.__del(b, WebGLBuffer); };
  P.isBuffer = function (b) { return this.__is(b, WebGLBuffer, 0); };
  P.bindBuffer = function (t, b) { const id = this.__id(b, WebGLBuffer); if (id >= 0) __glBindBuffer(this.__cid, t, id); };
  P.bufferData = function (t, data, usage, srcOffset, length) {
    if (typeof data === "number") return __glBufferData(this.__cid, t, data, usage, 0);
    if (data === null || data === undefined) return this.__err(0x501);
    if (!isView(data) && !(data instanceof ArrayBuffer)) throw new TypeError("bufferData: bad data");
    const es = isView(data) && data.BYTES_PER_ELEMENT ? data.BYTES_PER_ELEMENT : 1;
    __glBufferData(this.__cid, t, data, usage, (srcOffset || 0) * es, length ? length * es : undefined);
  };
  P.bufferSubData = function (t, offset, data, srcOffset, length) {
    if (data === null || data === undefined) return this.__err(0x501);
    const es = isView(data) && data.BYTES_PER_ELEMENT ? data.BYTES_PER_ELEMENT : 1;
    __glBufferSubData(this.__cid, t, offset, data, (srcOffset || 0) * es, (length || 0) * es);
  };
  P.getBufferParameter = function (t, p) { return __glGetBufferParam(this.__cid, t, p); };

  // ---- textures
  P.createTexture = function () { return this.__new(1); };
  P.deleteTexture = function (t) { this.__del(t, WebGLTexture); };
  P.isTexture = function (t) { return this.__is(t, WebGLTexture, 1); };
  P.bindTexture = function (target, t) { const id = this.__id(t, WebGLTexture); if (id >= 0) __glBindTexture(this.__cid, target, id); };
  P.activeTexture = function (t) { __glActiveTexture(this.__cid, t); };
  P.texParameteri = P.texParameterf = function (t, p, v) { __glTexParam(this.__cid, t, p, v); };
  P.getTexParameter = function (t, p) { return __glGetTexParam(this.__cid, t, p); };
  P.generateMipmap = function (t) { __glGenerateMipmap(this.__cid, t); };

  // An image source: [kind, key, data, w, h] for __glTexImageSource, or null to skip.
  function sourceOf(src) {
    if (src && src.__offscreen) src = src.__offscreen;
    if (src && src.__bitmap) src = src.__bitmap;
    if (typeof ImageData === "function" && src instanceof ImageData) return [2, "", src.data, src.width, src.height];
    if (src && typeof src === "object" && src.nodeType === 1) {
      const tag = src.localName;
      if (tag === "canvas") { if (g.__glHas(src.__id)) __glFlush(src.__id); return [1, String(src.__id), null, 0, 0]; }
      if (tag === "img" || tag === "image") return [0, String(src.__id), null, 0, 0];
      if (tag === "video") return null;
    }
    throw new TypeError("texImage: unsupported image source");
  }
  P.__texSource = function (target, level, internal, format, type, src, sub, offs, is3d) {
    const s = sourceOf(src);
    if (!s) return;
    __glTexImageSource(this.__cid, target, level, internal, format, type, s[0], s[1], s[2], s[3], s[4], !!sub, offs || [0, 0, 0], !!is3d);
  };
  P.texImage2D = function (target, level, internal, a, b, c, d, e, f, srcOffset) {
    if (arguments.length <= 6 || (arguments.length === 9 && !(f === null || f === undefined || isView(f) || typeof f === "number")) || (arguments.length >= 9 && f && typeof f === "object" && !isView(f))) {
      if (arguments.length <= 6) return this.__texSource(target, level, internal, a, b, c);
      return this.__texSource(target, level, internal, d, e, f);
    }
    // (target, level, internal, w, h, border, format, type, pixels | null, srcOffset)
    if (typeof f === "number") return this.__err(0x502); // pixel unpack buffers aren't supported
    __glTexImage(this.__cid, target, level, internal, a, b, 1, c, d, e, f || null, srcOffset || 0, 0);
  };
  P.texSubImage2D = function (target, level, x, y, a, b, c, d, e, srcOffset) {
    if (arguments.length <= 7) return this.__texSource(target, level, 0, a, b, c, true, [x, y, 0]);
    if (e && typeof e === "object" && !isView(e)) return this.__texSource(target, level, 0, c, d, e, true, [x, y, 0]);
    if (e === null || e === undefined) throw new TypeError("texSubImage2D: no pixels");
    if (typeof e === "number") return this.__err(0x502);
    __glTexImage(this.__cid, target, level, 0, a, b, 1, 0, c, d, e, srcOffset || 0, 1, [x, y, 0]);
  };
  P.texImage3D = function (target, level, internal, w, h, depth, border, format, type, src, srcOffset) {
    if (src && typeof src === "object" && !isView(src)) return this.__texSource(target, level, internal, format, type, src, false, null, true);
    if (typeof src === "number") return this.__err(0x502);
    __glTexImage(this.__cid, target, level, internal, w, h, depth, border, format, type, src || null, srcOffset || 0, 2);
  };
  P.texSubImage3D = function (target, level, x, y, z, w, h, depth, format, type, src, srcOffset) {
    if (src && typeof src === "object" && !isView(src)) return this.__texSource(target, level, 0, format, type, src, true, [x, y, z], true);
    if (typeof src === "number") return this.__err(0x502);
    __glTexImage(this.__cid, target, level, 0, w, h, depth, 0, format, type, src, srcOffset || 0, 1, [x, y, z]);
  };
  P.texStorage2D = function (t, levels, internal, w, h) { __glTexStorage(this.__cid, t, levels, internal, w, h, 1); };
  P.texStorage3D = function (t, levels, internal, w, h, d) { __glTexStorage(this.__cid, t, levels, internal, w, h, d); };
  P.copyTexImage2D = function (t, level, internal, x, y, w, h, border) {
    if (border) return this.__err(0x501);
    __glCopyTexImage(this.__cid, t, level, internal, x, y, w, h, false);
  };
  P.copyTexSubImage2D = function (t, level, xo, yo, x, y, w, h) { __glCopyTexImage(this.__cid, t, level, 0, x, y, w, h, true, [xo, yo, 0]); };
  P.copyTexSubImage3D = function (t, level, xo, yo, zo, x, y, w, h) { __glCopyTexImage(this.__cid, t, level, 0, x, y, w, h, true, [xo, yo, zo]); };
  const view = (v, off, len) => {
    if (!isView(v)) throw new TypeError("compressed data must be an ArrayBufferView");
    const u8 = new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
    const es = v.BYTES_PER_ELEMENT || 1;
    return off || len ? u8.subarray((off || 0) * es, len ? ((off || 0) + len) * es : undefined) : u8;
  };
  P.compressedTexImage2D = function (t, level, internal, w, h, border, data, off, len) {
    if (arguments.length < 7) throw new TypeError("compressedTexImage2D: not enough arguments");
    if (typeof data === "number") return this.__err(0x502);
    __glCompressedTexImage(this.__cid, t, level, internal, w, h, 1, border, view(data, off, len), false, [0, 0, 0]);
  };
  P.compressedTexSubImage2D = function (t, level, x, y, w, h, format, data, off, len) {
    if (arguments.length < 8) throw new TypeError("compressedTexSubImage2D: not enough arguments");
    if (typeof data === "number") return this.__err(0x502);
    __glCompressedTexImage(this.__cid, t, level, format, w, h, 1, 0, view(data, off, len), true, [x, y, 0]);
  };
  P.compressedTexImage3D = function (t, level, internal, w, h, d, border, data) {
    if (arguments.length < 8) throw new TypeError("compressedTexImage3D: not enough arguments");
    this.__err(0x500);
  };
  P.compressedTexSubImage3D = function (t, level, x, y, z, w, h, d, format, data) {
    if (arguments.length < 10) throw new TypeError("compressedTexSubImage3D: not enough arguments");
    this.__err(0x500);
  };

  // ---- samplers (WebGL 2)
  P.createSampler = function () { return this.__new(7); };
  P.deleteSampler = function (s) { this.__del(s, WebGLSampler); };
  P.isSampler = function (s) { return this.__is(s, WebGLSampler, 7); };
  P.bindSampler = function (unit, s) { const id = this.__id(s, WebGLSampler); if (id >= 0) __glBindSampler(this.__cid, unit, id); };
  P.samplerParameteri = P.samplerParameterf = function (s, p, v) { const id = this.__id(s, WebGLSampler); if (id > 0) __glSamplerParam(this.__cid, id, p, v); };
  P.getSamplerParameter = function (s, p) { const id = this.__id(s, WebGLSampler); return id > 0 ? __glGetSamplerParam(this.__cid, id, p) : null; };

  // ---- shaders and programs
  P.createShader = function (type) { return this.__new(4, type); };
  P.deleteShader = function (s) { this.__del(s, WebGLShader); };
  P.isShader = function (s) { return this.__is(s, WebGLShader, 4); };
  P.shaderSource = function (s, src) { const id = this.__id(s, WebGLShader); if (id > 0) __glShaderSource(this.__cid, id, String(src)); };
  P.compileShader = function (s) { const id = this.__id(s, WebGLShader); if (id > 0) __glCompileShader(this.__cid, id); };
  P.getShaderParameter = function (s, p) { const id = this.__id(s, WebGLShader, true); return id > 0 ? __glGetShaderParam(this.__cid, id, p) : null; };
  P.getShaderInfoLog = function (s) { const id = this.__id(s, WebGLShader, true); return id > 0 ? __glGetShaderInfoLog(this.__cid, id) : null; };
  P.getShaderSource = function (s) { const id = this.__id(s, WebGLShader, true); return id > 0 ? __glGetShaderSource(this.__cid, id) : null; };
  P.getShaderPrecisionFormat = function (st, pt) {
    if (pt >= 0x8df0 && pt <= 0x8df2) return new WebGLShaderPrecisionFormat(127, 127, 23);
    if (pt >= 0x8df3 && pt <= 0x8df5) return new WebGLShaderPrecisionFormat(31, 30, 0);
    this.__err(0x500);
    return null;
  };
  P.getTranslatedShaderSource = function (s) { return this.getShaderSource(s); };
  P.createProgram = function () { const p = this.__new(5); if (p) p.__gen = 0; return p; };
  P.deleteProgram = function (p) { this.__del(p, WebGLProgram); };
  P.isProgram = function (p) { return this.__is(p, WebGLProgram, 5); };
  P.attachShader = function (p, s) { const pi = this.__id(p, WebGLProgram), si = this.__id(s, WebGLShader); if (pi > 0 && si > 0) __glAttachShader(this.__cid, pi, si); };
  P.detachShader = function (p, s) { const pi = this.__id(p, WebGLProgram), si = this.__id(s, WebGLShader, true); if (pi > 0 && si > 0) __glDetachShader(this.__cid, pi, si); };
  P.linkProgram = function (p) { const id = this.__id(p, WebGLProgram); if (id > 0) { __glLinkProgram(this.__cid, id); p.__gen++; } };
  P.useProgram = function (p) { const id = this.__id(p, WebGLProgram); if (id >= 0) { __glUseProgram(this.__cid, id); this.__prog = p; } };
  P.validateProgram = function (p) { const id = this.__id(p, WebGLProgram); if (id > 0) __glValidateProgram(this.__cid, id); };
  P.getProgramParameter = function (p, pname) { const id = this.__id(p, WebGLProgram, true); return id > 0 ? __glGetProgramParam(this.__cid, id, pname) : null; };
  P.getProgramInfoLog = function (p) { const id = this.__id(p, WebGLProgram, true); return id > 0 ? __glGetProgramInfoLog(this.__cid, id) : null; };
  P.getAttachedShaders = function (p) {
    const id = this.__id(p, WebGLProgram);
    if (id <= 0) return null;
    const ids = __glGetAttachedShaders(this.__cid, id);
    return ids ? ids.map((x) => this.__obj(x)).filter(Boolean) : null;
  };
  P.bindAttribLocation = function (p, i, name) { const id = this.__id(p, WebGLProgram); if (id > 0) __glBindAttribLocation(this.__cid, id, i, String(name)); };
  P.getAttribLocation = function (p, name) { const id = this.__id(p, WebGLProgram); return id > 0 ? __glGetAttribLocation(this.__cid, id, String(name)) : -1; };
  P.getFragDataLocation = function (p, name) { const id = this.__id(p, WebGLProgram); return id > 0 ? __glGetFragDataLocation(this.__cid, id, String(name)) : -1; };
  P.getUniformLocation = function (p, name) {
    const id = this.__id(p, WebGLProgram);
    if (id <= 0) return null;
    const r = __glGetUniformLocation(this.__cid, id, String(name));
    if (!r) return null;
    const loc = new WebGLUniformLocation();
    Object.defineProperties(loc, { __ctx: { value: this }, __prog: { value: p }, __idx: { value: r[0] }, __el: { value: r[1] }, __gen: { value: p.__gen } });
    return loc;
  };
  P.getActiveUniform = function (p, i) {
    const id = this.__id(p, WebGLProgram);
    const r = id > 0 ? __glGetActive(this.__cid, id, i, false) : null;
    return r ? new WebGLActiveInfo(r[0], r[1], r[2]) : null;
  };
  P.getActiveAttrib = function (p, i) {
    const id = this.__id(p, WebGLProgram);
    const r = id > 0 ? __glGetActive(this.__cid, id, i, true) : null;
    return r ? new WebGLActiveInfo(r[0], r[1], r[2]) : null;
  };
  P.getUniform = function (p, loc) {
    const id = this.__id(p, WebGLProgram);
    if (id <= 0 || !loc) return null;
    if (loc.__prog !== p || loc.__gen !== p.__gen) { this.__err(0x502); return null; }
    return __glGetUniform(this.__cid, id, loc.__idx, loc.__el);
  };
  // Uniform upload: kind = base*100 + cols*10 + rows.
  P.__uniform = function (loc, kind, data, transpose) {
    if (loc === null || loc === undefined) return;
    if (!(loc instanceof WebGLUniformLocation)) throw new TypeError("uniform: not a WebGLUniformLocation");
    if (loc.__ctx !== this || loc.__prog !== this.__prog || loc.__gen !== loc.__prog.__gen) return this.__err(0x502);
    __glUniform(this.__cid, loc.__prog.__id, loc.__idx, loc.__el, kind, data, !!transpose);
  };
  const arrOf = (T, v, off, len) => {
    if (v === null || v === undefined || typeof v !== "object") throw new TypeError("uniform: expected an array");
    let a = v instanceof T ? v : T.from(v);
    off = off >>> 0;
    len = len >>> 0;
    // A source range outside the array is INVALID_VALUE (the caller reports it).
    if (off > a.length || (len && off + len > a.length)) return null;
    if (off || len) a = a.subarray(off, len ? off + len : undefined);
    return a;
  };
  for (let n = 1; n <= 4; n++) {
    for (const [suffix, base] of [["f", 0], ["i", 1], ["ui", 2]]) {
      const kind = base * 100 + 10 + n;
      const T = typedOf(base);
      P["uniform" + n + suffix] = function (loc, ...v) { this.__uniform(loc, kind, T.from(v.slice(0, n).map((x) => +x || 0))); };
      P["uniform" + n + suffix + "v"] = function (loc, v, off, len) {
        const a = arrOf(T, v, off, len);
        if (!a) return this.__err(0x501);
        if (loc == null) return;
        this.__uniform(loc, kind, a);
      };
    }
  }
  for (const [c, r] of [[2, 2], [3, 3], [4, 4], [2, 3], [2, 4], [3, 2], [3, 4], [4, 2], [4, 3]]) {
    const name = c === r ? "uniformMatrix" + c + "fv" : "uniformMatrix" + c + "x" + r + "fv";
    P[name] = function (loc, transpose, v, off, len) {
      const a = arrOf(Float32Array, v, off, len);
      if (!a) return this.__err(0x501);
      if (loc == null) return;
      this.__uniform(loc, c * 10 + r, a, transpose);
    };
  }
  // Uniform blocks (WebGL 2).
  P.getUniformBlockIndex = function (p, name) { const id = this.__id(p, WebGLProgram); return id > 0 ? __glUniformBlock(this.__cid, 0, id, String(name)) : 0xffffffff; };
  P.uniformBlockBinding = function (p, i, b) { const id = this.__id(p, WebGLProgram); if (id > 0) __glUniformBlock(this.__cid, 1, id, i, b); };
  P.getActiveUniformBlockName = function (p, i) { const id = this.__id(p, WebGLProgram); return id > 0 ? __glUniformBlock(this.__cid, 2, id, i) : null; };
  P.getActiveUniformBlockParameter = function (p, i, pname) {
    const id = this.__id(p, WebGLProgram);
    if (id <= 0) return null;
    const r = __glUniformBlock(this.__cid, 3, id, i, pname);
    return Array.isArray(r) ? Uint32Array.from(r) : r;
  };
  P.getUniformIndices = function (p, names) { const id = this.__id(p, WebGLProgram); return id > 0 ? __glUniformBlock(this.__cid, 4, id, Array.from(names, String)) : null; };
  P.getActiveUniforms = function (p, idx, pname) {
    const id = this.__id(p, WebGLProgram);
    if (id <= 0) return null;
    const r = __glUniformBlock(this.__cid, 5, id, Array.from(idx), pname);
    return r && pname === 0x8a3e ? r.map((x) => !!x) : r;
  };
  P.bindBufferBase = function (t, i, b) { const id = this.__id(b, WebGLBuffer); if (id >= 0) __glBindBufferRange(this.__cid, t, i, id, 0, 0); };
  P.bindBufferRange = function (t, i, b, off, size) { const id = this.__id(b, WebGLBuffer); if (id >= 0) __glBindBufferRange(this.__cid, t, i, id, off, size); };
  P.getIndexedParameter = function (t, i) { return null; };
  P.copyBufferSubData = function (rt, wt, ro, wo, size) { __glCopyBufferSubData(this.__cid, rt, wt, ro, wo, size); };
  P.getBufferSubData = function (t, off, dst, dstOffset, length) {
    const es = dst.BYTES_PER_ELEMENT || 1;
    __glGetBufferSubData(this.__cid, t, off, dst, (dstOffset || 0) * es, (length || 0) * es);
  };

  // ---- vertex attributes and drawing
  P.enableVertexAttribArray = function (i) { __glEnableVA(this.__cid, i, true); };
  P.disableVertexAttribArray = function (i) { __glEnableVA(this.__cid, i, false); };
  P.vertexAttribPointer = function (i, size, type, norm, stride, offset) { __glVAPointer(this.__cid, i, size, type, !!norm, stride, offset, false); };
  P.vertexAttribIPointer = function (i, size, type, stride, offset) { __glVAPointer(this.__cid, i, size, type, false, stride, offset, true); };
  P.vertexAttribDivisor = function (i, d) { __glVADivisor(this.__cid, i, d); };
  for (let n = 1; n <= 4; n++) {
    P["vertexAttrib" + n + "f"] = function (i, ...v) { __glVertexAttrib(this.__cid, i, v.slice(0, n).map(Number).concat([0, 0, 0, 1].slice(n)), 0); };
    P["vertexAttrib" + n + "fv"] = function (i, v) { const a = Array.from(v).slice(0, n); if (a.length < n) return this.__err(0x501); __glVertexAttrib(this.__cid, i, a.concat([0, 0, 0, 1].slice(n)), 0); };
  }
  P.vertexAttribI4i = function (i, x, y, z, w) { __glVertexAttrib(this.__cid, i, [x, y, z, w], 1); };
  P.vertexAttribI4ui = function (i, x, y, z, w) { __glVertexAttrib(this.__cid, i, [x, y, z, w], 2); };
  P.vertexAttribI4iv = function (i, v) { const a = Array.from(v).slice(0, 4); if (a.length < 4) return this.__err(0x501); __glVertexAttrib(this.__cid, i, a, 1); };
  P.vertexAttribI4uiv = function (i, v) { const a = Array.from(v).slice(0, 4); if (a.length < 4) return this.__err(0x501); __glVertexAttrib(this.__cid, i, a, 2); };
  P.getVertexAttrib = function (i, p) {
    const r = __glGetVertexAttrib(this.__cid, i, p);
    return p === 0x889f ? this.__obj(r) : r;
  };
  P.getVertexAttribOffset = function (i, p) { return p === 0x8645 ? __glGetVertexAttrib(this.__cid, i, p) : (this.__err(0x500), 0); };
  P.createVertexArray = function () { return this.__new(6); };
  P.deleteVertexArray = function (v) { this.__del(v, WebGLVertexArrayObject); };
  P.isVertexArray = function (v) { return this.__is(v, WebGLVertexArrayObject, 6); };
  P.bindVertexArray = function (v) { const id = this.__id(v, WebGLVertexArrayObject); if (id >= 0) __glBindVao(this.__cid, id); };
  P.__drawArrays = function (m, f, c, n) { if (!this.__lost) __glDrawArrays(this.__cid, m, f, c, n); };
  P.__drawElements = function (m, c, t, o, n) { if (!this.__lost) __glDrawElements(this.__cid, m, c, t, o, n); };
  P.drawArrays = function (m, f, c) { this.__drawArrays(m, f, c, 1); };
  P.drawElements = function (m, c, t, o) { this.__drawElements(m, c, t, o, 1); };
  P.drawArraysInstanced = function (m, f, c, n) { this.__drawArrays(m, f, c, n); };
  P.drawElementsInstanced = function (m, c, t, o, n) { this.__drawElements(m, c, t, o, n); };
  P.drawRangeElements = function (m, s, e, c, t, o) { this.__drawElements(m, c, t, o, 1); };
  P.clear = function (mask) { if (!this.__lost) __glClear(this.__cid, mask); };
  P.clearBufferfv = function (b, i, v, off) { __glClearBuffer(this.__cid, b, i, Array.from(v).slice(off || 0), 0); };
  P.clearBufferiv = function (b, i, v, off) { __glClearBuffer(this.__cid, b, i, Array.from(v).slice(off || 0), 1); };
  P.clearBufferuiv = function (b, i, v, off) { __glClearBuffer(this.__cid, b, i, Array.from(v).slice(off || 0), 2); };
  P.clearBufferfi = function (b, i, d, s) { __glClearBuffer(this.__cid, b, i, [d, s], 3); };

  // ---- fixed-function state
  P.enable = function (c) { __glEnable(this.__cid, c, true); };
  P.disable = function (c) { __glEnable(this.__cid, c, false); };
  P.isEnabled = function (c) { return __glIsEnabled(this.__cid, c); };
  P.clearColor = function (r, gg, b, a) { __glState(this.__cid, 0, +r, +gg, +b, +a); };
  P.clearDepth = function (d) { __glState(this.__cid, 1, +d); };
  P.clearStencil = function (s) { __glState(this.__cid, 2, s | 0); };
  P.blendColor = function (r, gg, b, a) { __glState(this.__cid, 3, +r, +gg, +b, +a); };
  P.blendEquation = function (m) { __glState(this.__cid, 4, m, m); };
  P.blendEquationSeparate = function (rgb, a) { __glState(this.__cid, 4, rgb, a); };
  P.blendFunc = function (s, d) { __glState(this.__cid, 5, s, d, s, d); };
  P.blendFuncSeparate = function (sr, dr, sa, da) { __glState(this.__cid, 5, sr, dr, sa, da); };
  P.colorMask = function (r, gg, b, a) { __glState(this.__cid, 6, r ? 1 : 0, gg ? 1 : 0, b ? 1 : 0, a ? 1 : 0); };
  P.cullFace = function (m) { __glState(this.__cid, 7, m); };
  P.frontFace = function (m) { __glState(this.__cid, 8, m); };
  P.depthFunc = function (f) { __glState(this.__cid, 9, f); };
  P.depthMask = function (f) { __glState(this.__cid, 10, f ? 1 : 0); };
  P.depthRange = function (n, f) { __glState(this.__cid, 11, +n, +f); };
  P.stencilFunc = function (f, r, m) { __glState(this.__cid, 12, 0x408, f, r, m >>> 0); };
  P.stencilFuncSeparate = function (face, f, r, m) { __glState(this.__cid, 12, face, f, r, m >>> 0); };
  P.stencilOp = function (a, b, c) { __glState(this.__cid, 13, 0x408, a, b, c); };
  P.stencilOpSeparate = function (face, a, b, c) { __glState(this.__cid, 13, face, a, b, c); };
  P.stencilMask = function (m) { __glState(this.__cid, 14, 0x408, m >>> 0); };
  P.stencilMaskSeparate = function (face, m) { __glState(this.__cid, 14, face, m >>> 0); };
  P.viewport = function (x, y, w, h) { __glState(this.__cid, 15, x | 0, y | 0, w | 0, h | 0); };
  P.scissor = function (x, y, w, h) { __glState(this.__cid, 16, x | 0, y | 0, w | 0, h | 0); };
  P.lineWidth = function (w) { __glState(this.__cid, 17, +w); };
  P.polygonOffset = function (f, u) { __glState(this.__cid, 18, +f, +u); };
  P.sampleCoverage = function (v, inv) { __glState(this.__cid, 19, +v, inv ? 1 : 0); };
  P.hint = function (t, m) { __glState(this.__cid, 20, t, m); };
  P.pixelStorei = function (p, v) { __glPixelStore(this.__cid, p, typeof v === "boolean" ? (v ? 1 : 0) : v); };
  P.getParameter = function (p) {
    if (this.__lost) return null;
    const r = __glGetParameter(this.__cid, p);
    if (OBJ_PARAMS.has(p)) return this.__obj(r);
    if (p === 0x86a3) return new Uint32Array(this.__compressed || []);
    if (p === 0x0c23 && Array.isArray(r)) return r;
    return r;
  };
  P.readPixels = function (x, y, w, h, format, type, dst, dstOffset) {
    if (typeof dst === "number") return this.__err(0x502);
    if (!isView(dst)) throw new TypeError("readPixels: pixels must be an ArrayBufferView");
    const ok = (type === 0x1401 && dst instanceof Uint8Array) || (type === 0x1406 && dst instanceof Float32Array) ||
      (type === 0x1404 && dst instanceof Int32Array) || (type === 0x1405 && dst instanceof Uint32Array) ||
      (type === 0x8363 || type === 0x8033 || type === 0x8034) && dst instanceof Uint16Array;
    if (!ok) return this.__err(0x502);
    __glReadPixels(this.__cid, x, y, w, h, format, type, dst, (dstOffset || 0) * (dst.BYTES_PER_ELEMENT || 1));
  };

  // ---- framebuffers and renderbuffers
  P.createFramebuffer = function () { return this.__new(2); };
  P.deleteFramebuffer = function (f) { this.__del(f, WebGLFramebuffer); };
  P.isFramebuffer = function (f) { return this.__is(f, WebGLFramebuffer, 2); };
  P.bindFramebuffer = function (t, f) { const id = this.__id(f, WebGLFramebuffer); if (id >= 0) __glBindFramebuffer(this.__cid, t, id); };
  P.framebufferTexture2D = function (t, att, textarget, tex, level) { const id = this.__id(tex, WebGLTexture); if (id >= 0) __glFramebufferTexture(this.__cid, t, att, id ? textarget : 0xde1, id, level, 0); };
  P.framebufferTextureLayer = function (t, att, tex, level, layer) { const id = this.__id(tex, WebGLTexture); if (id >= 0) __glFramebufferTexture(this.__cid, t, att, id ? 0 : 0xde1, id, level, layer); };
  P.framebufferRenderbuffer = function (t, att, rt, rb) { const id = this.__id(rb, WebGLRenderbuffer); if (id >= 0) __glFramebufferRenderbuffer(this.__cid, t, att, rt, id); };
  P.checkFramebufferStatus = function (t) { return __glCheckFramebufferStatus(this.__cid, t); };
  P.getFramebufferAttachmentParameter = function (t, att, p) {
    const r = __glGetFramebufferAttachmentParam(this.__cid, t, att, p);
    return Array.isArray(r) ? this.__obj(r[1]) : r;
  };
  P.createRenderbuffer = function () { return this.__new(3); };
  P.deleteRenderbuffer = function (r) { this.__del(r, WebGLRenderbuffer); };
  P.isRenderbuffer = function (r) { return this.__is(r, WebGLRenderbuffer, 3); };
  P.bindRenderbuffer = function (t, r) { const id = this.__id(r, WebGLRenderbuffer); if (id >= 0) __glBindRenderbuffer(this.__cid, t, id); };
  P.renderbufferStorage = function (t, f, w, h) { __glRenderbufferStorage(this.__cid, t, 0, f, w, h); };
  P.renderbufferStorageMultisample = function (t, s, f, w, h) { __glRenderbufferStorage(this.__cid, t, s, f, w, h); };
  P.getRenderbufferParameter = function (t, p) { return __glGetRenderbufferParam(this.__cid, t, p); };
  P.getInternalformatParameter = function (t, f, p) { return p === 0x80a9 ? new Int32Array([]) : null; };
  P.drawBuffers = function (b) { __glDrawBuffers(this.__cid, Array.from(b)); };
  P.readBuffer = function (s) { __glReadBuffer(this.__cid, s); };
  P.blitFramebuffer = function (...a) { __glBlitFramebuffer(this.__cid, ...a); };
  P.invalidateFramebuffer = P.invalidateSubFramebuffer = function () {};

  // ---- queries, syncs and transform feedback (WebGL 2): complete immediately
  P.createQuery = function () { return this.__new(8); };
  P.deleteQuery = function (q) { this.__del(q, WebGLQuery); };
  P.isQuery = function (q) { return this.__is(q, WebGLQuery, 8); };
  P.beginQuery = function (t, q) { this.__query = q; };
  P.endQuery = function () { this.__query = null; };
  P.getQuery = function () { return this.__query || null; };
  P.getQueryParameter = function (q, p) { return p === 0x8867 ? true : 0; };
  P.fenceSync = function () { return this.__new(9); };
  P.deleteSync = function (s) { this.__del(s, WebGLSync); };
  P.isSync = function (s) { return this.__is(s, WebGLSync, 9); };
  P.clientWaitSync = function () { return 0x911a; };
  P.waitSync = function () {};
  P.getSyncParameter = function (s, p) { return p === 0x9114 ? 0x9119 : p === 0x9112 ? 0x9116 : p === 0x9113 ? 0x9117 : 0; };
  P.createTransformFeedback = function () { const o = new WebGLTransformFeedback(); o.__deleted = false; return o; };
  P.deleteTransformFeedback = function () {};
  P.isTransformFeedback = function (t) { return t instanceof WebGLTransformFeedback; };
  P.bindTransformFeedback = P.beginTransformFeedback = P.endTransformFeedback = P.pauseTransformFeedback = P.resumeTransformFeedback = function () {};
  P.transformFeedbackVaryings = function () {};
  P.getTransformFeedbackVarying = function () { return null; };

  // ---- the two context classes
  function WebGLRenderingContext() { throw new TypeError("Illegal constructor"); }
  function WebGL2RenderingContext() { throw new TypeError("Illegal constructor"); }
  WebGLRenderingContext.prototype = Object.create(P);
  WebGL2RenderingContext.prototype = Object.create(P);
  for (const [k, v] of Object.entries(K1)) {
    for (const C of [WebGLRenderingContext, WebGL2RenderingContext]) {
      Object.defineProperty(C, k, { value: v, enumerable: true });
      Object.defineProperty(C.prototype, k, { value: v, enumerable: true });
    }
  }
  for (const [k, v] of Object.entries(K2)) {
    Object.defineProperty(WebGL2RenderingContext, k, { value: v, enumerable: true });
    Object.defineProperty(WebGL2RenderingContext.prototype, k, { value: v, enumerable: true });
  }
  // WebGL 2-only methods aren't on WebGL 1 contexts.
  const only2 = ["texImage3D", "texSubImage3D", "texStorage2D", "texStorage3D", "copyTexSubImage3D", "compressedTexImage3D",
    "compressedTexSubImage3D", "createSampler", "deleteSampler", "isSampler", "bindSampler", "samplerParameteri", "samplerParameterf",
    "getSamplerParameter", "getFragDataLocation", "getUniformBlockIndex", "uniformBlockBinding", "getActiveUniformBlockName",
    "getActiveUniformBlockParameter", "getUniformIndices", "getActiveUniforms", "bindBufferBase", "bindBufferRange",
    "getIndexedParameter", "copyBufferSubData", "getBufferSubData", "vertexAttribIPointer", "vertexAttribDivisor", "vertexAttribI4i",
    "vertexAttribI4ui", "vertexAttribI4iv", "vertexAttribI4uiv", "createVertexArray", "deleteVertexArray", "isVertexArray",
    "bindVertexArray", "drawArraysInstanced", "drawElementsInstanced", "drawRangeElements", "clearBufferfv", "clearBufferiv",
    "clearBufferuiv", "clearBufferfi", "renderbufferStorageMultisample", "getInternalformatParameter", "drawBuffers", "readBuffer",
    "blitFramebuffer", "invalidateFramebuffer", "invalidateSubFramebuffer", "framebufferTextureLayer", "createQuery", "deleteQuery",
    "isQuery", "beginQuery", "endQuery", "getQuery", "getQueryParameter", "fenceSync", "deleteSync", "isSync", "clientWaitSync",
    "waitSync", "getSyncParameter", "createTransformFeedback", "deleteTransformFeedback", "isTransformFeedback",
    "bindTransformFeedback", "beginTransformFeedback", "endTransformFeedback", "pauseTransformFeedback", "resumeTransformFeedback",
    "transformFeedbackVaryings", "getTransformFeedbackVarying", "uniformMatrix2x3fv", "uniformMatrix2x4fv", "uniformMatrix3x2fv",
    "uniformMatrix3x4fv", "uniformMatrix4x2fv", "uniformMatrix4x3fv"];
  for (const n of [1, 2, 3, 4]) only2.push("uniform" + n + "ui", "uniform" + n + "uiv");
  for (const name of only2) {
    Object.defineProperty(WebGLRenderingContext.prototype, name, { value: undefined, writable: true, configurable: true });
  }
  Object.assign(g, { WebGLRenderingContext, WebGL2RenderingContext });

  // ---- hooks for canvas.js
  g.__webglGetContext = function (canvas, type, attrs) {
    const v2 = type === "webgl2";
    let c = ctxs.get(canvas.__id);
    if (c) return c.__v2 === v2 ? c : null;
    c = Object.create(v2 ? WebGL2RenderingContext.prototype : WebGLRenderingContext.prototype);
    c.__init(canvas, attrs, v2);
    ctxs.set(canvas.__id, c);
    return c;
  };
  g.__webglHas = (id) => ctxs.has(id);
  g.__glFlushCanvas = (id) => { if (ctxs.has(id)) __glFlush(id); };
  g.__glResizeCanvas = (el) => {
    const c = ctxs.get(el.__id);
    if (c) __glResize(el.__id, c.__w(), c.__h());
  };
})();
