// Editor metadata attributes, surfaced to the editor via
// `Reflection::field_user_attributes`. Slang reflects only attributes
// declared as `{Name}Attribute` structs with `[__AttributeUsage(...)]`; an
// undeclared `[Foo]` stays an internal unchecked attribute and is invisible
// to reflection. Shaders that carry editor metadata `import` this module,
// so the attribute structs are `public` — `internal` would hide them behind
// the module boundary.
module editor_metadata;

[__AttributeUsage(_AttributeTargets.Var)]
public struct EditorColorAttribute
{
};

[__AttributeUsage(_AttributeTargets.Var)]
public struct RangeAttribute
{
    int min;
    int max;
};
