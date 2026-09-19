package sdk

import (
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"math/big"
	"reflect"
	"strconv"
	"strings"
	"unicode/utf8"
)

type codecType struct {
	Kind, Name string
	Inner      *codecType
}
type codecField struct {
	Name, Wire string
	Required   bool
	Type       codecType
}
type codecVariant struct {
	Name string
	Root int
	Type codecType
}
type codecModel struct {
	Kind     string
	Root     int
	NonNull  bool
	Source   ValidationSource
	Type     codecType
	Fields   []codecField
	Extras   *codecType
	Variants []codecVariant
}
type codecProgram struct {
	Models             map[string]codecModel
	MaxDepth, MaxSteps int
	JSON               Limits
	Validation         struct{ Version, Profile string }
}

//go:embed codec-plan.json
var codecPlanData []byte
var codecPlan = loadCodecPlan()

func loadCodecPlan() *codecProgram {
	var plan codecProgram
	if err := json.Unmarshal(codecPlanData, &plan); err != nil {
		panic("invalid generated codec metadata")
	}
	if plan.Validation.Version != checkedValidation.Version || plan.Validation.Profile != checkedValidation.Profile {
		panic("codec and validation profiles do not match")
	}
	for _, model := range plan.Models {
		if model.Root < 0 || model.Root >= len(checkedValidation.Nodes) || checkedValidation.Nodes[model.Root].Source != model.Source {
			panic("codec root does not match its validation source")
		}
	}
	return &plan
}

// CodecError is a located native conversion or resource failure.
type CodecError struct {
	Kind          string
	Source        ValidationSource
	Path, Message string
	Cause         error
}

func (e *CodecError) Error() string {
	return "codec " + e.Kind + " at " + e.Source.Document + "#" + e.Source.Pointer + " " + e.Path + ": " + e.Message
}
func (e *CodecError) Unwrap() error         { return e.Cause }
func (e *CodecError) ResourceLimited() bool { return e.Kind == "resource" }

type codecContext struct {
	validation *validationSession
	remaining  int
	source     ValidationSource
}

func newCodecContext(name string) *codecContext {
	return &codecContext{validation: newValidationSession(), remaining: codecPlan.MaxSteps, source: codecPlan.Models[name].Source}
}
func (c *codecContext) issue(kind, path, message string) error {
	return &CodecError{Kind: kind, Source: c.source, Path: path, Message: message}
}
func (c *codecContext) step(path string, depth, cost int) error {
	if cost < 0 || cost > c.remaining || depth > codecPlan.MaxDepth {
		return c.issue("resource", path, "conversion budget exhausted")
	}
	c.remaining -= cost
	return nil
}

// String values already have a conversion visit; object names add one. Charge
// their bytes before scanning, and admit the same Unicode domain at every
// native/value boundary, before an invalid string or key can escape in Value.
func (c *codecContext) stringWork(text, path string, depth, visits int) error {
	if err := c.step(path, depth, visits+len(text)); err != nil {
		return err
	}
	if !utf8.ValidString(text) {
		const message = "string is not valid UTF-8"
		return &CodecError{
			Kind: "conversion", Source: c.source, Path: path, Message: message,
			Cause: &JSONError{Kind: JSONBadUnicode, Offset: -1, Message: message},
		}
	}
	return nil
}

func (c *codecContext) check(root int, value Value) error {
	if err := c.validation.Check(root, value); err != nil {
		var failure *ValidationError
		if errors.As(err, &failure) {
			kind := "invalid"
			if failure.Kind == "evaluation_failure" {
				kind = "resource"
			}
			return &CodecError{kind, failure.Source, failure.InstancePath, failure.Message, err}
		}
		return err
	}
	return nil
}

// Account for the complete copy before JSON encoding/parsing allocates it. All
// clones in one codec call consume this context's remaining conversion work.
func (c *codecContext) cloneJSON(value Value, path string, depth int) (Value, error) {
	if err := c.jsonWork(value, path, depth, map[containerIdentity]struct{}{}); err != nil {
		return nil, err
	}
	data, err := Encode(value, codecPlan.JSON)
	if err != nil {
		return nil, err
	}
	return Parse(data, codecPlan.JSON)
}
func (c *codecContext) jsonWork(value Value, path string, depth int, active map[containerIdentity]struct{}) error {
	if err := c.step(path, depth, 1); err != nil {
		return err
	}
	// Match the exact JSON runtime's active-path identity: shared acyclic values
	// are copied and charged again; only a back-edge is a cycle.
	switch value.(type) {
	case []Value, map[string]Value:
		native := reflect.ValueOf(value)
		if native.Len() != 0 {
			id := containerIdentity{kind: native.Kind(), pointer: native.Pointer()}
			if native.Kind() == reflect.Slice {
				id.length = native.Len()
			}
			if _, found := active[id]; found {
				return &JSONError{Kind: JSONCycle, Offset: -1, Message: "cyclic JSON value"}
			}
			active[id] = struct{}{}
			defer delete(active, id)
		}
	}
	switch value := value.(type) {
	case string:
		return c.stringWork(value, path, depth, 0)
	case Number:
		return c.step(path, depth, len(value.String()))
	case Integer:
		return c.step(path, depth, len(value.String()))
	case []Value:
		for index, item := range value {
			if err := c.jsonWork(item, validationChild(path, strconv.Itoa(index)), depth+1, active); err != nil {
				return err
			}
		}
	case map[string]Value:
		for name, item := range value {
			if err := c.stringWork(name, path, depth, 1); err != nil {
				return err
			}
			if err := c.jsonWork(item, validationChild(path, name), depth+1, active); err != nil {
				return err
			}
		}
	}
	return nil
}

// Codec preserves the native model's type and validates every public boundary.
type Codec[T any] struct{ name string }

func (codec Codec[T]) Decode(data []byte) (T, error) {
	var zero T
	value, err := Parse(data, codecPlan.JSON)
	if err != nil {
		return zero, err
	}
	return codec.DecodeValue(value)
}
func (codec Codec[T]) DecodeValue(value Value) (T, error) {
	var zero T
	context := newCodecContext(codec.name)
	value, err := context.cloneJSON(value, "", 0)
	if err != nil {
		return zero, err
	}
	result, err := context.decodeModel(codec.name, value, "", 0)
	if err != nil {
		return zero, err
	}
	return result.Interface().(T), nil
}
func (codec Codec[T]) Encode(value T) ([]byte, error) {
	wire, err := codec.EncodeValue(value)
	if err != nil {
		return nil, err
	}
	return Encode(wire, codecPlan.JSON)
}
func (codec Codec[T]) EncodeValue(value T) (Value, error) {
	native := reflect.ValueOf(&value).Elem()
	return newCodecContext(codec.name).encodeModel(codec.name, native, "", 0)
}

func (c *codecContext) decodeModel(name string, value Value, path string, depth int) (reflect.Value, error) {
	if err := c.step(path, depth, 1); err != nil {
		return reflect.Value{}, err
	}
	model := codecPlan.Models[name]
	prior := c.source
	c.source = model.Source
	defer func() { c.source = prior }()
	if model.NonNull && value == nil {
		return reflect.Value{}, c.issue("conversion", path, "null cannot inhabit a non-null representation")
	}
	// V3 validates the whole selected graph once. Revalidating a nested model
	// as a new root would discard its caller's dynamic resource context.
	if codecPlan.Validation.Version != "suspect.validation.experimental.v3" || depth == 0 {
		if err := c.check(model.Root, value); err != nil {
			return reflect.Value{}, err
		}
	}
	actual := codecNativeTypes[name]
	switch model.Kind {
	case "alias", "literal":
		return c.decodeType(model.Type, actual, value, path, depth+1)
	case "union":
		for _, variant := range model.Variants {
			err := c.validation.Check(variant.Root, value)
			if err != nil {
				var failure *ValidationError
				if errors.As(err, &failure) && failure.Kind == "invalid" {
					continue
				}
				return reflect.Value{}, err
			}
			wrapper := reflect.New(codecNativeTypes[variant.Name]).Elem()
			decoded, err := c.decodeType(variant.Type, wrapper.Field(0).Type(), value, path, depth+1)
			if err != nil {
				return reflect.Value{}, err
			}
			wrapper.Field(0).Set(decoded)
			out := reflect.New(actual).Elem()
			out.Set(wrapper)
			return out, nil
		}
		return reflect.Value{}, c.issue("conversion", path, "no source union branch matched")
	case "object":
		object, ok := value.(map[string]Value)
		if !ok {
			return reflect.Value{}, c.issue("conversion", path, "expected object")
		}
		out := reflect.New(actual).Elem()
		known := map[string]bool{}
		for _, field := range model.Fields {
			known[field.Wire] = true
			item, present := object[field.Wire]
			if !present {
				if field.Required {
					return reflect.Value{}, c.issue("conversion", path, "required field absent")
				}
				continue
			}
			target := out.FieldByName(field.Name)
			converted, err := c.decodeType(field.Type, target.Type(), item, validationChild(path, field.Wire), depth+1)
			if err != nil {
				return reflect.Value{}, err
			}
			target.Set(converted)
		}
		for key, item := range object {
			if known[key] {
				continue
			}
			if model.Extras == nil {
				return reflect.Value{}, c.issue("conversion", path, "extra field on closed model")
			}
			if err := c.stringWork(key, path, depth, 1); err != nil {
				return reflect.Value{}, err
			}
			setter := out.Addr().MethodByName("SetExtra")
			converted, err := c.decodeType(*model.Extras, setter.Type().In(1), item, validationChild(path, key), depth+1)
			if err != nil {
				return reflect.Value{}, err
			}
			result := setter.Call([]reflect.Value{reflect.ValueOf(key), converted})
			if !result[0].IsNil() {
				return reflect.Value{}, result[0].Interface().(error)
			}
		}
		return out, nil
	default:
		return reflect.Value{}, c.issue("conversion", path, "unknown model descriptor")
	}
}

func (c *codecContext) encodeModel(name string, value reflect.Value, path string, depth int) (Value, error) {
	if err := c.step(path, depth, 1); err != nil {
		return nil, err
	}
	model := codecPlan.Models[name]
	prior := c.source
	c.source = model.Source
	defer func() { c.source = prior }()
	var wire Value
	var err error
	switch model.Kind {
	case "alias", "literal":
		wire, err = c.encodeType(model.Type, value, path, depth+1)
	case "union":
		if value.Kind() == reflect.Interface {
			if value.IsNil() {
				return nil, c.issue("conversion", path, "nil union")
			}
			value = value.Elem()
		}
		if value.Kind() == reflect.Pointer {
			if value.IsNil() {
				return nil, c.issue("conversion", path, "nil union wrapper")
			}
			value = value.Elem()
		}
		matched := false
		for _, variant := range model.Variants {
			if value.Type() == codecNativeTypes[variant.Name] {
				wire, err = c.encodeType(variant.Type, value.Field(0), path, depth+1)
				if err == nil {
					err = c.check(variant.Root, wire)
				}
				matched = true
				break
			}
		}
		if !matched {
			return nil, c.issue("conversion", path, "unknown native union variant")
		}
	case "object":
		if value.Type() != codecNativeTypes[name] {
			return nil, c.issue("conversion", path, "wrong native model type")
		}
		object := map[string]Value{}
		known := map[string]bool{}
		for _, field := range model.Fields {
			known[field.Wire] = true
			item := value.FieldByName(field.Name)
			if !field.Required && !item.FieldByName("IsSet").Bool() {
				if !item.FieldByName("Value").IsZero() || field.Type.Kind == "presence" && item.FieldByName("Null").Bool() {
					return nil, c.issue("conversion", path, "inconsistent absent wrapper")
				}
				continue
			}
			converted, e := c.encodeType(field.Type, item, validationChild(path, field.Wire), depth+1)
			if e != nil {
				return nil, e
			}
			object[field.Wire] = converted
		}
		if model.Extras != nil {
			copy := reflect.New(value.Type())
			copy.Elem().Set(value)
			extra := copy.MethodByName("Extra").Call(nil)[0]
			entries := extra.MapRange()
			for entries.Next() {
				name := entries.Key().String()
				if known[name] {
					return nil, c.issue("conversion", path, "colliding extra key")
				}
				if e := c.stringWork(name, path, depth, 1); e != nil {
					return nil, e
				}
				converted, e := c.encodeType(*model.Extras, entries.Value(), validationChild(path, name), depth+1)
				if e != nil {
					return nil, e
				}
				object[name] = converted
			}
		}
		wire = object
	default:
		return nil, c.issue("conversion", path, "unknown model descriptor")
	}
	if err != nil {
		return nil, err
	}
	if model.NonNull && wire == nil {
		return nil, c.issue("conversion", path, "null cannot inhabit a non-null representation")
	}
	if codecPlan.Validation.Version != "suspect.validation.experimental.v3" || depth == 0 {
		if err = c.check(model.Root, wire); err != nil {
			return nil, err
		}
	}
	return wire, nil
}

func (c *codecContext) decodeType(ty codecType, native reflect.Type, value Value, path string, depth int) (reflect.Value, error) {
	if err := c.step(path, depth, 1); err != nil {
		return reflect.Value{}, err
	}
	out := reflect.New(native).Elem()
	switch ty.Kind {
	case "named":
		return c.decodeModel(ty.Name, value, path, depth+1)
	case "nullable", "optional", "presence":
		if ty.Kind == "optional" || ty.Kind == "presence" {
			out.FieldByName("IsSet").SetBool(true)
		}
		if value == nil {
			if ty.Kind == "optional" {
				return reflect.Value{}, c.issue("conversion", path, "null optional nonnull value")
			}
			if ty.Kind == "presence" {
				out.FieldByName("Null").SetBool(true)
			}
			return out, nil
		}
		if ty.Kind == "nullable" {
			out.FieldByName("IsValue").SetBool(true)
		}
		inner, err := c.decodeType(*ty.Inner, out.FieldByName("Value").Type(), value, path, depth+1)
		if err != nil {
			return reflect.Value{}, err
		}
		out.FieldByName("Value").Set(inner)
		return out, nil
	case "pointer":
		if value == nil {
			return reflect.Value{}, c.issue("conversion", path, "null recursive pointer")
		}
		inner, err := c.decodeType(*ty.Inner, native.Elem(), value, path, depth+1)
		if err != nil {
			return reflect.Value{}, err
		}
		out = reflect.New(native.Elem())
		out.Elem().Set(inner)
		return out, nil
	case "slice":
		array, ok := value.([]Value)
		if !ok {
			return reflect.Value{}, c.issue("conversion", path, "expected array")
		}
		out = reflect.MakeSlice(native, len(array), len(array))
		for i, item := range array {
			converted, err := c.decodeType(*ty.Inner, native.Elem(), item, validationChild(path, strconv.Itoa(i)), depth+1)
			if err != nil {
				return reflect.Value{}, err
			}
			out.Index(i).Set(converted)
		}
		return out, nil
	case "map":
		object, ok := value.(map[string]Value)
		if !ok {
			return reflect.Value{}, c.issue("conversion", path, "expected object map")
		}
		out = reflect.MakeMapWithSize(native, len(object))
		for name, item := range object {
			if err := c.stringWork(name, path, depth, 1); err != nil {
				return reflect.Value{}, err
			}
			converted, err := c.decodeType(*ty.Inner, native.Elem(), item, validationChild(path, name), depth+1)
			if err != nil {
				return reflect.Value{}, err
			}
			out.SetMapIndex(reflect.ValueOf(name), converted)
		}
		return out, nil
	case "primitive":
		if ty.Name == "Value" {
			if value == nil {
				return out, nil
			}
			out.Set(reflect.ValueOf(value))
			return out, nil
		}
		if ty.Name == "Number" || ty.Name == "Integer" {
			token := ""
			switch number := value.(type) {
			case Number:
				token = number.String()
			case Integer:
				token = number.String()
			}
			if ty.Name == "Number" {
				number, err := ParseNumber(token)
				if err != nil {
					return reflect.Value{}, err
				}
				return reflect.ValueOf(number), nil
			}
			number, err := ParseInteger(token)
			if err != nil {
				return reflect.Value{}, err
			}
			return reflect.ValueOf(number), nil
		}
		switch native.Kind() {
		case reflect.String:
			text, ok := value.(string)
			if !ok {
				return reflect.Value{}, c.issue("conversion", path, "expected string")
			}
			if err := c.stringWork(text, path, depth, 0); err != nil {
				return reflect.Value{}, err
			}
			out.SetString(text)
		case reflect.Bool:
			truth, ok := value.(bool)
			if !ok {
				return reflect.Value{}, c.issue("conversion", path, "expected boolean")
			}
			out.SetBool(truth)
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64, reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
			number, err := c.validation.number(value, c.source, path)
			if err != nil {
				return reflect.Value{}, err
			}
			if !number.integral() {
				return reflect.Value{}, c.issue("conversion", path, "fractional integer")
			}
			if number.sign == 0 {
				return out, nil
			}
			if number.exponent.Cmp(big.NewInt(20)) > 0 {
				return reflect.Value{}, c.issue("conversion", path, "integer out of native range")
			}
			text := number.digits + strings.Repeat("0", int(number.exponent.Int64()))
			if number.sign < 0 {
				text = "-" + text
			}
			if native.Kind() >= reflect.Uint && native.Kind() <= reflect.Uint64 {
				integer, err := strconv.ParseUint(text, 10, native.Bits())
				if err != nil {
					return reflect.Value{}, c.issue("conversion", path, "integer out of native range")
				}
				out.SetUint(integer)
			} else {
				integer, err := strconv.ParseInt(text, 10, native.Bits())
				if err != nil {
					return reflect.Value{}, c.issue("conversion", path, "integer out of native range")
				}
				out.SetInt(integer)
			}
		default:
			return reflect.Value{}, c.issue("conversion", path, "unsupported scalar representation")
		}
		return out, nil
	default:
		return reflect.Value{}, c.issue("conversion", path, "unknown type descriptor")
	}
}
func (c *codecContext) encodeType(ty codecType, value reflect.Value, path string, depth int) (Value, error) {
	if err := c.step(path, depth, 1); err != nil {
		return nil, err
	}
	switch ty.Kind {
	case "named":
		return c.encodeModel(ty.Name, value, path, depth+1)
	case "nullable", "optional", "presence":
		empty := ty.Kind == "nullable" && !value.FieldByName("IsValue").Bool() || ty.Kind == "presence" && value.FieldByName("Null").Bool()
		if (ty.Kind == "optional" || ty.Kind == "presence") && !value.FieldByName("IsSet").Bool() {
			return nil, c.issue("conversion", path, "absent wrapper outside an optional field")
		}
		if empty {
			if !value.FieldByName("Value").IsZero() {
				return nil, c.issue("conversion", path, "inconsistent null wrapper")
			}
			return nil, nil
		}
		inner, err := c.encodeType(*ty.Inner, value.FieldByName("Value"), path, depth+1)
		if err != nil {
			return nil, err
		}
		if inner == nil {
			return nil, c.issue("conversion", path, "null cannot inhabit a present non-null wrapper value")
		}
		return inner, nil
	case "pointer":
		if value.IsNil() {
			return nil, c.issue("conversion", path, "nil non-null pointer")
		}
		return c.encodeType(*ty.Inner, value.Elem(), path, depth+1)
	case "slice":
		out := make([]Value, value.Len())
		for i := 0; i < value.Len(); i++ {
			item, err := c.encodeType(*ty.Inner, value.Index(i), validationChild(path, strconv.Itoa(i)), depth+1)
			if err != nil {
				return nil, err
			}
			out[i] = item
		}
		return out, nil
	case "map":
		out := map[string]Value{}
		entries := value.MapRange()
		for entries.Next() {
			name := entries.Key().String()
			if err := c.stringWork(name, path, depth, 1); err != nil {
				return nil, err
			}
			item, err := c.encodeType(*ty.Inner, entries.Value(), validationChild(path, name), depth+1)
			if err != nil {
				return nil, err
			}
			out[name] = item
		}
		return out, nil
	case "primitive":
		if ty.Name == "Value" {
			return c.cloneJSON(value.Interface(), path, depth)
		}
		if ty.Name == "Number" {
			number := value.Interface().(Number)
			if _, err := ParseNumber(number.String()); err != nil {
				return nil, err
			}
			return number, nil
		}
		if ty.Name == "Integer" {
			number := value.Interface().(Integer)
			if _, err := ParseInteger(number.String()); err != nil {
				return nil, err
			}
			return number, nil
		}
		switch value.Kind() {
		case reflect.String:
			text := value.String()
			if err := c.stringWork(text, path, depth, 0); err != nil {
				return nil, err
			}
			return text, nil
		case reflect.Bool:
			return value.Bool(), nil
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
			return ParseNumber(strconv.FormatInt(value.Int(), 10))
		case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
			return ParseNumber(strconv.FormatUint(value.Uint(), 10))
		}
	}
	return nil, c.issue("conversion", path, fmt.Sprintf("unsupported type instruction %s", ty.Kind))
}
