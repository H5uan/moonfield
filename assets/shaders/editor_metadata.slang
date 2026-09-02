// Editor metadata attributes, surfaced to the editor via
// `Reflection::field_user_attributes`. Slang reflects only attributes
// declared as `{Name}Attribute` structs with `[__AttributeUsage(...)]`; an
// undeclared `[Foo]` stays an internal unchecked attribute and is invisible
// to reflection. Shaders that carry editor metadata include this file.
[__AttributeUsage(_AttributeTargets.Var)]
struct EditorColorAttribute
{
};

[__AttributeUsage(_AttributeTargets.Var)]
struct RangeAttribute
{
    int min;
    int max;
};