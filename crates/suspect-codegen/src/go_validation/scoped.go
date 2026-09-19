// Scoped evaluated locations for suspect.validation.experimental.v2.
package sdk

import (
	"reflect"
	"sort"
	"strconv"
	"unicode/utf8"
)

type validationLocations struct {
	properties map[string]struct{}
	items      map[int]struct{}
}

func newValidationLocations() validationLocations {
	return validationLocations{map[string]struct{}{}, map[int]struct{}{}}
}

type validationEvaluated struct {
	valid     bool
	locations validationLocations
}
type validationIdentity struct {
	node      int
	path      string
	temporary uint64
	kind      reflect.Kind
	pointer   uintptr
	length    int
	context   int
}
type validationScoped struct {
	session   *validationSession
	findings  []ValidationFinding
	active    map[validationIdentity]bool
	numbers   map[string]validationDecimal
	temporary uint64
	resources *validationResources
}

func (s *validationSession) checkScoped(root int, value Value) error {
	state := &validationScoped{session: s, active: map[validationIdentity]bool{}, numbers: map[string]validationDecimal{}}
	if s.program.ResourceContext != nil {
		state.resources = newValidationResources()
	}
	result, err := state.eval(root, value, "", 0, 0)
	if err != nil {
		return err
	}
	if result.valid {
		return nil
	}
	if len(state.findings) == 0 {
		return s.issue("invalid", s.program.Nodes[root].Source, "", "schema rejected the value")
	}
	first := state.findings[0]
	return &ValidationError{Kind: "invalid", Source: first.Source, InstancePath: first.InstancePath, Message: first.Message, Findings: state.findings}
}
func (v *validationScoped) emit(source ValidationSource, path, message string) bool {
	cap := v.session.program.Limits.MaxErrors
	if cap == 0 || len(v.findings) < cap {
		v.findings = append(v.findings, ValidationFinding{Source: source, InstancePath: path, Message: message})
	}
	return false
}
func (v *validationScoped) step(source ValidationSource, path string) error {
	return v.session.spend(source, path, 1)
}
func validationKeys[T any](object map[string]T) []string {
	keys := make([]string, 0, len(object))
	for key := range object {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}
func (v *validationScoped) merge(into *validationLocations, other validationLocations, source ValidationSource, path string) error {
	for _, key := range validationKeys(other.properties) {
		if err := v.step(source, path); err != nil {
			return err
		}
		into.properties[key] = struct{}{}
	}
	items := make([]int, 0, len(other.items))
	for index := range other.items {
		items = append(items, index)
	}
	sort.Ints(items)
	for _, index := range items {
		if err := v.step(source, path); err != nil {
			return err
		}
		into.items[index] = struct{}{}
	}
	return nil
}
func (v *validationScoped) eval(index int, value Value, path string, depth int, temporary uint64) (validationEvaluated, error) {
	empty := validationEvaluated{}
	if index < 0 || index >= len(v.session.program.Nodes) {
		return empty, v.session.issue("evaluation_failure", ValidationSource{}, path, "invalid schema target")
	}
	node := v.session.program.Nodes[index]
	if err := v.step(node.Source, path); err != nil {
		return empty, err
	}
	if depth >= v.session.program.Limits.MaxDepth {
		return empty, v.session.issue("evaluation_failure", node.Source, path, "recursive evaluation/depth budget exhausted")
	}
	leave, err := v.enterResource(index, node.Source, path)
	if err != nil {
		return empty, err
	}
	defer leave()
	identity := validationIdentity{node: index, path: path, temporary: temporary}
	if v.resources != nil {
		identity.context = v.resources.context
	}
	switch value.(type) {
	case map[string]Value, []Value:
		native := reflect.ValueOf(value)
		if native.Len() > 0 {
			identity.path = ""
			identity.kind = native.Kind()
			identity.pointer = native.Pointer()
			if native.Kind() == reflect.Slice {
				identity.length = native.Len()
			}
		}
	}
	if v.active[identity] {
		return empty, v.session.issue("evaluation_failure", node.Source, path, "recursive evaluation/depth budget exhausted")
	}
	if validationKind(value) == "invalid" {
		return empty, v.session.issue("evaluation_failure", node.Source, path, "instance is not an exact JSON value")
	}
	if text, ok := value.(string); ok && !utf8.ValidString(text) {
		return empty, v.session.issue("evaluation_failure", node.Source, path, "invalid Unicode instance")
	}
	v.active[identity] = true
	defer delete(v.active, identity)
	result := validationEvaluated{valid: true, locations: newValidationLocations()}
	for _, check := range node.Checks {
		if err := v.step(check.Source, path); err != nil {
			return empty, err
		}
		ok, produced, err := v.apply(node, check, value, path, depth, temporary, &result.locations)
		if err != nil {
			return empty, err
		}
		result.valid = result.valid && ok
		if ok {
			if err = v.merge(&result.locations, produced, check.Source, path); err != nil {
				return empty, err
			}
		}
	}
	if !result.valid {
		result.locations = validationLocations{}
	}
	return result, nil
}
func (v *validationScoped) trial(index int, value Value, path string, depth int, temporary uint64) (validationEvaluated, error) {
	saved := v.findings
	v.findings = nil
	result, err := v.eval(index, value, path, depth, temporary)
	v.findings = saved
	return result, err
}
func (v *validationScoped) number(value Value, source ValidationSource, path string) (validationDecimal, error) {
	var token string
	switch n := value.(type) {
	case Number:
		token = n.String()
	case Integer:
		token = n.String()
	default:
		return validationDecimal{}, v.session.issue("evaluation_failure", source, path, "invalid numeric instance")
	}
	if cached, ok := v.numbers[token]; ok {
		return cached, nil
	}
	number, err := v.session.number(value, source, path)
	if err == nil {
		v.numbers[token] = number
	}
	return number, err
}
func validationSourceChild(source ValidationSource, name string) ValidationSource {
	source.Pointer = validationChild(source.Pointer, name)
	return source
}
func validationHas(names []string, name string) bool {
	for _, n := range names {
		if n == name {
			return true
		}
	}
	return false
}

func (v *validationScoped) apply(node validationNode, check validationCheck, value Value, path string, depth int, temporary uint64, local *validationLocations) (bool, validationLocations, error) {
	produced := newValidationLocations()
	ok := true
	bad := func(source ValidationSource, message string) { ok = v.emit(source, path, message) }
	child := func(target int, value Value, childPath string) (validationEvaluated, error) {
		return v.eval(target, value, childPath, depth+1, temporary)
	}
	switch check.Op {
	case "always":
		if !check.truth {
			bad(check.Source, "value is rejected by a false schema")
		}
	case "type":
		kind := validationKind(value)
		ok = validationHas(check.Types, kind)
		if !ok && kind == "number" && validationHas(check.Types, "integer") {
			n, err := v.number(value, check.Source, path)
			if err != nil {
				return false, produced, err
			}
			ok = n.integral()
		}
		if !ok {
			bad(check.Source, "instance does not match the declared type")
		}
	case "ref":
		result, err := child(check.target, value, path)
		return result.valid, result.locations, err
	case "dynamicRef":
		target, err := v.dynamicTarget(check, path)
		if err != nil {
			return false, produced, err
		}
		result, err := child(target, value, path)
		return result.valid, result.locations, err
	case "properties":
		if object, yes := value.(map[string]Value); yes {
			for _, p := range check.Properties {
				if err := v.step(check.Source, path); err != nil {
					return false, produced, err
				}
				item, exists := object[p.Name]
				if !exists {
					continue
				}
				result, err := child(p.Target, item, validationChild(path, p.Name))
				if err != nil {
					return false, produced, err
				}
				ok = ok && result.valid
				produced.properties[p.Name] = struct{}{}
			}
		}
	case "required":
		if object, yes := value.(map[string]Value); yes {
			for _, name := range check.Names {
				if err := v.step(check.Source, path); err != nil {
					return false, produced, err
				}
				if _, exists := object[name]; !exists {
					bad(check.Source, "required property is absent: "+name)
				}
			}
		}
	case "additionalProperties", "additionalPropertiesWithPatterns", "patternProperties", "propertyNames", "unevaluatedProperties":
		return v.object(check, node, value, path, depth, temporary, local)
	case "items", "prefixItems", "contains", "unevaluatedItems":
		return v.array(check, node, value, path, depth, temporary, local)
	case "allOf", "anyOf", "oneOf":
		var passing []validationLocations
		matches := 0
		for _, target := range check.Targets {
			if err := v.step(check.Source, path); err != nil {
				return false, produced, err
			}
			var result validationEvaluated
			var err error
			if check.Op == "allOf" {
				result, err = child(target, value, path)
			} else {
				result, err = v.trial(target, value, path, depth+1, temporary)
			}
			if err != nil {
				return false, produced, err
			}
			if result.valid {
				matches++
				passing = append(passing, result.locations)
			}
		}
		if check.Op == "allOf" {
			ok = matches == len(check.Targets)
		} else {
			ok = matches > 0
			if check.Op == "oneOf" {
				ok = matches == 1
			}
			if !ok {
				bad(check.Source, "composition matched "+strconv.Itoa(matches)+" alternatives")
			}
		}
		if ok {
			for _, locations := range passing {
				if err := v.merge(&produced, locations, check.Source, path); err != nil {
					return false, produced, err
				}
			}
		}
	case "not":
		result, err := v.trial(check.target, value, path, depth+1, temporary)
		if err != nil {
			return false, produced, err
		}
		if result.valid {
			bad(check.Source, "instance matches the negated schema")
		}
	case "if":
		condition, err := v.trial(check.Condition, value, path, depth+1, temporary)
		if err != nil {
			return false, produced, err
		}
		target := check.ElseTarget
		if condition.valid {
			target = check.ThenTarget
			if err = v.merge(local, condition.locations, check.Source, path); err != nil {
				return false, produced, err
			}
		}
		if target != nil {
			result, err := child(*target, value, path)
			return result.valid, result.locations, err
		}
	case "dependentRequired":
		if object, yes := value.(map[string]Value); yes {
			for _, dependency := range check.requiredDependencies {
				if err := v.step(check.Source, path); err != nil {
					return false, produced, err
				}
				if _, exists := object[dependency.name]; !exists {
					continue
				}
				for _, name := range dependency.required {
					if err := v.step(check.Source, path); err != nil {
						return false, produced, err
					}
					if _, exists := object[name]; !exists {
						bad(validationSourceChild(check.Source, dependency.name), "required property is absent: "+name)
					}
				}
			}
		}
	case "dependentSchemas":
		if object, yes := value.(map[string]Value); yes {
			var passing []validationLocations
			for _, dependency := range check.dependent {
				if err := v.step(check.Source, path); err != nil {
					return false, produced, err
				}
				if _, exists := object[dependency.Name]; !exists {
					continue
				}
				result, err := child(dependency.Target, value, path)
				if err != nil {
					return false, produced, err
				}
				ok = ok && result.valid
				if result.valid {
					passing = append(passing, result.locations)
				}
			}
			if ok {
				for _, locations := range passing {
					if err := v.merge(&produced, locations, check.Source, path); err != nil {
						return false, produced, err
					}
				}
			}
		}
	case "bound", "multipleOf":
		if validationKind(value) == "number" {
			a, err := v.number(value, check.Source, path)
			if err != nil {
				return false, produced, err
			}
			b, err := validationNumber(check.operand)
			if err != nil {
				return false, produced, v.session.issue("evaluation_failure", check.Source, path, "invalid compiled numeric operand")
			}
			if check.Op == "bound" {
				order := a.compare(b)
				if check.maximum && order > 0 || !check.maximum && order < 0 || check.Exclusive && order == 0 {
					bad(check.Source, "numeric value violates bound")
				}
			} else {
				divisible, err := a.divisible(b, func(int) error { return nil })
				if err != nil {
					return false, produced, err
				}
				if !divisible {
					bad(check.Source, "numeric value is not a multiple of divisor")
				}
			}
		}
	case "count":
		if validationKind(value) == check.countTarget {
			size := 0
			switch x := value.(type) {
			case string:
				size = utf8.RuneCountInString(x)
			case []Value:
				size = len(x)
			case map[string]Value:
				size = len(x)
			}
			a, _ := validationNumber(strconv.Itoa(size))
			b, err := validationNumber(check.operand)
			if err != nil {
				return false, produced, v.session.issue("evaluation_failure", check.Source, path, "invalid count operand")
			}
			order := a.compare(b)
			if check.maximum && order > 0 || !check.maximum && order < 0 {
				bad(check.Source, "instance cardinality violates bound")
			}
		}
	case "const":
		same, err := v.equal(value, check.constant, check.Source, path)
		if err != nil {
			return false, produced, err
		}
		if !same {
			bad(check.Source, "instance does not equal the const value")
		}
	case "enum":
		ok = false
		for _, literal := range check.literals {
			if err := v.step(check.Source, path); err != nil {
				return false, produced, err
			}
			same, err := v.equal(value, literal, check.Source, path)
			if err != nil {
				return false, produced, err
			}
			if same {
				ok = true
				break
			}
		}
		if !ok {
			bad(check.Source, "instance does not equal any enum value")
		}
	case "uniqueItems":
		if array, yes := value.([]Value); yes {
		outer:
			for i, item := range array {
				if err := v.step(check.Source, path); err != nil {
					return false, produced, err
				}
				for _, previous := range array[:i] {
					if err := v.step(check.Source, path); err != nil {
						return false, produced, err
					}
					same, err := v.equal(item, previous, check.Source, path)
					if err != nil {
						return false, produced, err
					}
					if same {
						bad(check.Source, "array contains equal items")
						break outer
					}
				}
			}
		}
	case "pattern":
		if text, yes := value.(string); yes {
			matched, err := validationScopedPattern(&check.Program, text, func() error { return v.step(check.Source, path) })
			if err != nil {
				return false, produced, err
			}
			if !matched {
				bad(check.Source, "string does not match pattern")
			}
		}
	default:
		return false, produced, v.session.issue("evaluation_failure", check.Source, path, "unknown scoped instruction")
	}
	return ok, produced, nil
}

func (v *validationScoped) object(check validationCheck, node validationNode, value Value, path string, depth int, temporary uint64, local *validationLocations) (bool, validationLocations, error) {
	produced := newValidationLocations()
	object, yes := value.(map[string]Value)
	if !yes {
		return true, produced, nil
	}
	ok := true
	patterns := check.patterns
	if check.Op == "additionalPropertiesWithPatterns" {
		for _, adjacent := range node.Checks {
			if adjacent.Op == "patternProperties" {
				patterns = adjacent.patterns
				break
			}
		}
	}
	for _, name := range validationKeys(object) {
		if err := v.step(check.Source, path); err != nil {
			return false, produced, err
		}
		if !utf8.ValidString(name) {
			return false, produced, v.session.issue("evaluation_failure", check.Source, path, "invalid Unicode property name")
		}
		itemPath := validationChild(path, name)
		if check.Op == "propertyNames" {
			v.temporary++
			result, err := v.eval(check.target, name, itemPath, depth+1, v.temporary)
			if err != nil {
				return false, produced, err
			}
			ok = ok && result.valid
			continue
		}
		if check.Op == "unevaluatedProperties" {
			if _, marked := local.properties[name]; marked {
				continue
			}
		}
		if check.Op == "additionalProperties" || check.Op == "additionalPropertiesWithPatterns" {
			if validationHas(check.Declared, name) {
				continue
			}
		}
		if check.Op == "patternProperties" || check.Op == "additionalPropertiesWithPatterns" {
			matched := false
			for _, pattern := range patterns {
				if err := v.step(check.Source, path); err != nil {
					return false, produced, err
				}
				match, err := validationScopedPattern(&pattern.program, name, func() error { return v.step(check.Source, path) })
				if err != nil {
					return false, produced, err
				}
				if !match {
					continue
				}
				matched = true
				if check.Op == "additionalPropertiesWithPatterns" {
					break
				}
				result, err := v.eval(pattern.target, object[name], itemPath, depth+1, temporary)
				if err != nil {
					return false, produced, err
				}
				ok = ok && result.valid
				produced.properties[name] = struct{}{}
			}
			if check.Op == "patternProperties" || matched {
				continue
			}
		}
		result, err := v.eval(check.target, object[name], itemPath, depth+1, temporary)
		if err != nil {
			return false, produced, err
		}
		ok = ok && result.valid
		produced.properties[name] = struct{}{}
	}
	return ok, produced, nil
}

func (v *validationScoped) array(check validationCheck, node validationNode, value Value, path string, depth int, temporary uint64, local *validationLocations) (bool, validationLocations, error) {
	produced := newValidationLocations()
	array, yes := value.([]Value)
	if !yes {
		return true, produced, nil
	}
	ok := true
	matched := 0
	for index, item := range array {
		target := check.target
		if check.Op == "items" && index < check.Start {
			continue
		}
		if check.Op == "prefixItems" {
			if index >= len(check.Targets) {
				break
			}
			target = check.Targets[index]
		}
		if err := v.step(check.Source, path); err != nil {
			return false, produced, err
		}
		if check.Op == "unevaluatedItems" {
			if _, marked := local.items[index]; marked {
				continue
			}
		}
		itemPath := validationChild(path, strconv.Itoa(index))
		if check.Op == "contains" {
			result, err := v.trial(target, item, itemPath, depth+1, temporary)
			if err != nil {
				return false, produced, err
			}
			if result.valid {
				matched++
				produced.items[index] = struct{}{}
			}
			continue
		}
		result, err := v.eval(target, item, itemPath, depth+1, temporary)
		if err != nil {
			return false, produced, err
		}
		ok = ok && result.valid
		produced.items[index] = struct{}{}
	}
	if check.Op == "contains" {
		// The default 1 is semantic. Explicit counts were checked at compilation and
		// do not consume instance numeric-byte work or become invented operands.
		lower := matched >= 1
		zeroMinimum := false
		count, _ := validationNumber(strconv.Itoa(matched))
		if check.Minimum != nil {
			minimum, err := validationNumber(*check.Minimum)
			if err != nil {
				return false, produced, v.session.issue("evaluation_failure", check.Source, path, "invalid minimum count")
			}
			lower = count.compare(minimum) >= 0
			zeroMinimum = minimum.sign == 0
		}
		if matched > 0 || zeroMinimum {
			if err := v.merge(local, produced, check.Source, path); err != nil {
				return false, produced, err
			}
		}
		produced = newValidationLocations()
		if !lower {
			source := check.Source
			if check.Minimum != nil {
				source = validationSourceChild(node.Source, "minContains")
			}
			ok = v.emit(source, path, "array has fewer contains matches than required")
		}
		if check.maximumCount != nil {
			maximum, err := validationNumber(*check.maximumCount)
			if err != nil {
				return false, produced, v.session.issue("evaluation_failure", check.Source, path, "invalid maximum count")
			}
			if count.compare(maximum) > 0 {
				ok = v.emit(validationSourceChild(node.Source, "maxContains"), path, "array has more contains matches than allowed")
			}
		}
	}
	return ok, produced, nil
}

// Equality keeps a single shared allowance, including speculative branches and
// preflight space for all pending child comparisons, as in the checked profile.
func (v *validationScoped) equal(a, b Value, source ValidationSource, path string) (bool, error) {
	type pair struct {
		a, b  Value
		depth int
	}
	pending := []pair{{a, b, 0}}
	fail := func() error { return v.session.issue("evaluation_failure", source, path, "equality budget exhausted") }
	for len(pending) > 0 {
		if v.session.equality <= 0 {
			return false, fail()
		}
		v.session.equality--
		p := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if p.depth > v.session.program.Limits.MaxDepth {
			return false, fail()
		}
		kind := validationKind(p.a)
		if kind != validationKind(p.b) {
			return false, nil
		}
		switch left := p.a.(type) {
		case nil:
		case bool:
			if left != p.b.(bool) {
				return false, nil
			}
		case string:
			if left != p.b.(string) {
				return false, nil
			}
		case Number, Integer:
			a, err := v.number(p.a, source, path)
			if err != nil {
				return false, err
			}
			b, err := v.number(p.b, source, path)
			if err != nil {
				return false, err
			}
			if a.compare(b) != 0 {
				return false, nil
			}
		case []Value:
			right := p.b.([]Value)
			if len(left) != len(right) {
				return false, nil
			}
			if len(left) > v.session.equality-len(pending) {
				return false, fail()
			}
			for i := len(left) - 1; i >= 0; i-- {
				pending = append(pending, pair{left[i], right[i], p.depth + 1})
			}
		case map[string]Value:
			right := p.b.(map[string]Value)
			if len(left) != len(right) {
				return false, nil
			}
			if len(left) > v.session.equality-len(pending) {
				return false, fail()
			}
			keys := validationKeys(left)
			for i := len(keys) - 1; i >= 0; i-- {
				key := keys[i]
				other, exists := right[key]
				if !exists {
					return false, nil
				}
				pending = append(pending, pair{left[key], other, p.depth + 1})
			}
		default:
			return false, v.session.issue("evaluation_failure", source, path, "invalid exact JSON equality operand")
		}
	}
	return true, nil
}
