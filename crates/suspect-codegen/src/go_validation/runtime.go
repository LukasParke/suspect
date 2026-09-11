// Checked portable schema execution, with three outcomes and exact numeric data.
package sdk

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

// ValidationSource is an original schema/keyword identity.
type ValidationSource struct {
	Document string
	Pointer  string
}

// ValidationError distinguishes completed invalidity from incomplete work.
type ValidationError struct {
	Kind         string
	Source       ValidationSource
	InstancePath string
	Message      string
	Findings     []ValidationFinding
}

// ValidationFinding is one completed mismatch. Scoped evaluations retain ordered
// findings up to MaxErrors; evaluation failures remain a distinct outcome.
type ValidationFinding struct {
	Source       ValidationSource
	InstancePath string
	Message      string
}

func (e *ValidationError) Error() string {
	return "validation " + e.Kind + " at " + e.Source.Document + "#" + e.Source.Pointer + " " + e.InstancePath + ": " + e.Message
}
func (e *ValidationError) ResourceLimited() bool { return e.Kind == "evaluation_failure" }

type validationProgram struct {
	Version, Profile string
	Roots            []validationRoot
	Nodes            []validationNode
	Limits           validationLimits
	ResourceContext  *validationResourceContext
}
type validationLimits struct{ MaxDepth, MaxErrors, MaxNumberBytes, MaxEqualitySteps, MaxEvaluationSteps int }
type validationRoot struct {
	Target int
	Source ValidationSource
}
type validationNode struct {
	Source ValidationSource
	Checks []validationCheck
}
type validationProperty struct {
	Name   string
	Target int
}
type validationCheck struct {
	Source                 ValidationSource
	Op                     string
	Value                  json.RawMessage
	Values                 []json.RawMessage
	Target                 json.RawMessage
	Targets                []int
	Types, Names, Declared []string
	Properties             []validationProperty
	Start                  int
	Maximum                json.RawMessage
	Exclusive              bool
	Condition              int
	InitialResource        int
	Anchor                 *string
	ThenTarget, ElseTarget *int
	Minimum                *string
	Dependencies           json.RawMessage
	Patterns               [][]json.RawMessage
	Program                validationPattern
	maximum                bool
	maximumCount           *string
	dependent              []validationProperty
	requiredDependencies   []validationDependency
	patterns               []validationPatternProperty
	target                 int
	countTarget            string
	operand                string
	constant               Value
	literals               []Value
	truth                  bool
}
type validationDependency struct {
	name     string
	required []string
}
type validationPatternProperty struct {
	text    string
	program validationPattern
	target  int
}

//go:embed validation_program.json
var validationData []byte
var checkedValidation = loadValidationProgram()

func loadValidationProgram() *validationProgram {
	var program validationProgram
	if err := json.Unmarshal(validationData, &program); err != nil {
		panic("invalid generated validation metadata")
	}
	v1 := program.Version == "suspect.validation.experimental.v1" && program.Profile == "oas31-jsonschema202012-static-subset"
	v2 := program.Version == "suspect.validation.experimental.v2" && program.Profile == "oas31-jsonschema202012-static-applicators"
	v3 := program.Version == "suspect.validation.experimental.v3" && program.Profile == "oas31-jsonschema202012-resources-dynamic"
	if !v1 && !v2 && !v3 {
		panic("unsupported validation profile")
	}
	if (program.ResourceContext != nil) != v3 {
		panic("validation resource context/profile mismatch")
	}
	for i := range program.Nodes {
		for j := range program.Nodes[i].Checks {
			check := &program.Nodes[i].Checks[j]
			if validationScopedOpcode(check.Op) && !v2 && !v3 {
				panic("scoped instruction in v1 validation program")
			}
			switch check.Op {
			case "ref", "dynamicRef", "additionalProperties", "items", "not", "additionalPropertiesWithPatterns", "propertyNames", "unevaluatedProperties", "unevaluatedItems", "contains":
				if check.Op == "dynamicRef" && !v3 {
					panic("dynamic instruction in static validation program")
				}
				if err := json.Unmarshal(check.Target, &check.target); err != nil {
					panic("invalid generated target")
				}
				if check.Op == "contains" {
					if err := json.Unmarshal(check.Maximum, &check.maximumCount); err != nil {
						panic("invalid contains maximum")
					}
				}
			case "count":
				if err := json.Unmarshal(check.Target, &check.countTarget); err != nil {
					panic("invalid generated count target")
				}
				fallthrough
			case "bound", "multipleOf":
				if err := json.Unmarshal(check.Value, &check.operand); err != nil {
					panic("invalid generated numeric operand")
				}
				if len(check.Maximum) > 0 {
					if err := json.Unmarshal(check.Maximum, &check.maximum); err != nil {
						panic("invalid generated maximum flag")
					}
				}
			case "always":
				if err := json.Unmarshal(check.Value, &check.truth); err != nil {
					panic("invalid generated truth")
				}
			case "const":
				value, err := Parse(check.Value, DefaultLimits())
				if err != nil {
					panic("invalid generated literal")
				}
				check.constant = value
			case "enum":
				for _, raw := range check.Values {
					value, err := Parse(raw, DefaultLimits())
					if err != nil {
						panic("invalid generated enum literal")
					}
					check.literals = append(check.literals, value)
				}
			case "dependentSchemas":
				if err := json.Unmarshal(check.Dependencies, &check.dependent); err != nil {
					panic("invalid dependent schema operands")
				}
			case "dependentRequired":
				var tuples [][]json.RawMessage
				if err := json.Unmarshal(check.Dependencies, &tuples); err != nil {
					panic("invalid dependent required operands")
				}
				for _, tuple := range tuples {
					if len(tuple) != 2 {
						panic("invalid dependency tuple")
					}
					var dependency validationDependency
					if json.Unmarshal(tuple[0], &dependency.name) != nil || json.Unmarshal(tuple[1], &dependency.required) != nil {
						panic("invalid dependency values")
					}
					check.requiredDependencies = append(check.requiredDependencies, dependency)
				}
			case "patternProperties":
				for _, tuple := range check.Patterns {
					if len(tuple) != 3 {
						panic("invalid pattern property tuple")
					}
					var property validationPatternProperty
					if json.Unmarshal(tuple[0], &property.text) != nil || json.Unmarshal(tuple[1], &property.program) != nil || json.Unmarshal(tuple[2], &property.target) != nil {
						panic("invalid pattern property operands")
					}
					check.patterns = append(check.patterns, property)
				}
			case "if", "type", "properties", "required", "prefixItems", "allOf", "anyOf", "oneOf", "uniqueItems", "pattern":
			default:
				panic("unknown validation instruction")
			}
		}
	}
	if v3 {
		checkValidationResources(&program)
	}
	return &program
}
func validationScopedOpcode(op string) bool {
	switch op {
	case "if", "dependentRequired", "dependentSchemas", "contains", "patternProperties", "additionalPropertiesWithPatterns", "propertyNames", "unevaluatedProperties", "unevaluatedItems":
		return true
	}
	return false
}

type validationActive struct {
	node int
	path string
}
type validationSession struct {
	program                  *validationProgram
	steps, equality, numeric int
	active                   map[validationActive]bool
}

func newValidationSession() *validationSession {
	return &validationSession{program: checkedValidation, steps: checkedValidation.Limits.MaxEvaluationSteps, equality: checkedValidation.Limits.MaxEqualitySteps, numeric: checkedValidation.Limits.MaxEvaluationSteps, active: map[validationActive]bool{}}
}

// Validate checks a selected root node index with a fresh finite evaluation budget.
func Validate(root int, value Value) error { return newValidationSession().Check(root, value) }
func (s *validationSession) Check(root int, value Value) error {
	selected := false
	for _, entry := range s.program.Roots {
		if entry.Target == root {
			selected = true
			break
		}
	}
	if !selected {
		return &ValidationError{Kind: "evaluation_failure", Message: "root was not selected"}
	}
	if s.program.Version == "suspect.validation.experimental.v2" || s.program.Version == "suspect.validation.experimental.v3" {
		return s.checkScoped(root, value)
	}
	return s.evaluate(root, value, "", 0)
}
func (s *validationSession) issue(kind string, source ValidationSource, path, message string) *ValidationError {
	return &ValidationError{Kind: kind, Source: source, InstancePath: path, Message: message}
}
func (s *validationSession) spend(source ValidationSource, path string, amount int) error {
	s.steps -= amount
	if s.steps < 0 {
		return s.issue("evaluation_failure", source, path, "evaluation work budget exhausted")
	}
	return nil
}
func validationChild(path, key string) string {
	return path + "/" + strings.ReplaceAll(strings.ReplaceAll(key, "~", "~0"), "/", "~1")
}
func validationKind(value Value) string {
	switch value.(type) {
	case nil:
		return "null"
	case bool:
		return "boolean"
	case string:
		return "string"
	case Number, Integer:
		return "number"
	case []Value:
		return "array"
	case map[string]Value:
		return "object"
	default:
		return "invalid"
	}
}
func (s *validationSession) number(value Value, source ValidationSource, path string) (validationDecimal, error) {
	var token string
	switch v := value.(type) {
	case Number:
		token = v.String()
	case Integer:
		token = v.String()
	case string:
		token = v
	default:
		return validationDecimal{}, s.issue("evaluation_failure", source, path, "invalid numeric representation")
	}
	if len(token) > s.program.Limits.MaxNumberBytes {
		return validationDecimal{}, s.issue("evaluation_failure", source, path, "numeric operand budget exhausted")
	}
	number, err := validationNumber(token)
	if err != nil {
		return validationDecimal{}, s.issue("evaluation_failure", source, path, "invalid exact number")
	}
	return number, nil
}
func (s *validationSession) evaluate(index int, value Value, path string, depth int) error {
	if index < 0 || index >= len(s.program.Nodes) {
		return &ValidationError{Kind: "evaluation_failure", Message: "invalid target"}
	}
	node := s.program.Nodes[index]
	if err := s.spend(node.Source, path, 1); err != nil {
		return err
	}
	key := validationActive{index, path}
	if depth >= s.program.Limits.MaxDepth || s.active[key] {
		return s.issue("evaluation_failure", node.Source, path, "recursive evaluation/depth budget exhausted")
	}
	s.active[key] = true
	defer delete(s.active, key)
	var first error
	for _, check := range node.Checks {
		if err := s.spend(check.Source, path, 1); err != nil {
			return err
		}
		err := s.instruction(check, value, path, depth)
		if failure, ok := err.(*ValidationError); ok && failure.Kind == "evaluation_failure" {
			return err
		}
		if first == nil && err != nil {
			first = err
		}
	}
	return first
}
func (s *validationSession) instruction(check validationCheck, value Value, path string, depth int) error {
	bad := func() error { return s.issue("invalid", check.Source, path, check.Op+" assertion rejected the value") }
	conj := func(targets []validationProperty, object map[string]Value) error {
		var first error
		for _, property := range targets {
			if err := s.spend(check.Source, path, 1); err != nil {
				return err
			}
			child, exists := object[property.Name]
			if !exists {
				continue
			}
			err := s.evaluate(property.Target, child, validationChild(path, property.Name), depth+1)
			if failure, ok := err.(*ValidationError); ok && failure.Kind == "evaluation_failure" {
				return err
			}
			if first == nil && err != nil {
				first = err
			}
		}
		return first
	}
	switch check.Op {
	case "always":
		if !check.truth {
			return bad()
		}
	case "type":
		kind := validationKind(value)
		for _, allowed := range check.Types {
			if kind == allowed {
				return nil
			}
			if kind == "number" && allowed == "integer" {
				number, err := s.number(value, check.Source, path)
				if err != nil {
					return err
				}
				if number.integral() {
					return nil
				}
			}
		}
		return bad()
	case "ref":
		return s.evaluate(check.target, value, path, depth+1)
	case "properties":
		if object, ok := value.(map[string]Value); ok {
			return conj(check.Properties, object)
		}
	case "additionalProperties":
		if object, ok := value.(map[string]Value); ok {
			var properties []validationProperty
			for name := range object {
				declared := false
				for _, key := range check.Declared {
					if key == name {
						declared = true
						break
					}
				}
				if !declared {
					properties = append(properties, validationProperty{name, check.target})
				}
			}
			return conj(properties, object)
		}
	case "required":
		if object, ok := value.(map[string]Value); ok {
			for _, name := range check.Names {
				if err := s.spend(check.Source, path, 1); err != nil {
					return err
				}
				if _, exists := object[name]; !exists {
					return s.issue("invalid", check.Source, path, "required property is absent: "+name)
				}
			}
		}
	case "items", "prefixItems":
		if array, ok := value.([]Value); ok {
			var first error
			for i, item := range array {
				target := check.target
				if check.Op == "items" && i < check.Start {
					continue
				}
				if check.Op == "prefixItems" {
					if i >= len(check.Targets) {
						break
					}
					target = check.Targets[i]
				}
				if err := s.spend(check.Source, path, 1); err != nil {
					return err
				}
				err := s.evaluate(target, item, validationChild(path, strconv.Itoa(i)), depth+1)
				if failure, ok := err.(*ValidationError); ok && failure.Kind == "evaluation_failure" {
					return err
				}
				if first == nil && err != nil {
					first = err
				}
			}
			return first
		}
	case "allOf", "anyOf", "oneOf":
		matches := 0
		for _, target := range check.Targets {
			if err := s.spend(check.Source, path, 1); err != nil {
				return err
			}
			err := s.evaluate(target, value, path, depth+1)
			if failure, ok := err.(*ValidationError); ok && failure.Kind == "evaluation_failure" {
				return err
			}
			if err == nil {
				matches++
			}
		}
		if check.Op == "allOf" && matches != len(check.Targets) || check.Op == "anyOf" && matches == 0 || check.Op == "oneOf" && matches != 1 {
			return bad()
		}
	case "not":
		err := s.evaluate(check.target, value, path, depth+1)
		if failure, ok := err.(*ValidationError); ok && failure.Kind == "evaluation_failure" {
			return err
		}
		if err == nil {
			return bad()
		}
	case "bound", "multipleOf":
		if validationKind(value) == "number" {
			a, err := s.number(value, check.Source, path)
			if err != nil {
				return err
			}
			b, err := s.number(check.operand, check.Source, path)
			if err != nil {
				return err
			}
			if check.Op == "bound" {
				order := a.compare(b)
				if check.maximum && order > 0 || !check.maximum && order < 0 || check.Exclusive && order == 0 {
					return bad()
				}
			} else {
				ok, err := a.divisible(b, func(cost int) error {
					s.numeric -= cost
					if s.numeric < 0 {
						return s.issue("evaluation_failure", check.Source, path, "numeric work budget exhausted")
					}
					return nil
				})
				if err != nil {
					return err
				}
				if !ok {
					return bad()
				}
			}
		}
	case "count":
		if validationKind(value) == check.countTarget {
			size := 0
			switch v := value.(type) {
			case string:
				size = utf8.RuneCountInString(v)
			case []Value:
				size = len(v)
			case map[string]Value:
				size = len(v)
			}
			a, _ := validationNumber(strconv.Itoa(size))
			b, err := s.number(check.operand, check.Source, path)
			if err != nil {
				return err
			}
			order := a.compare(b)
			if check.maximum && order > 0 || !check.maximum && order < 0 {
				return bad()
			}
		}
	case "const":
		same, err := s.equal(value, check.constant, check.Source, path, 0)
		if err != nil {
			return err
		}
		if !same {
			return bad()
		}
	case "enum":
		for _, literal := range check.literals {
			same, err := s.equal(value, literal, check.Source, path, 0)
			if err != nil {
				return err
			}
			if same {
				return nil
			}
		}
		return bad()
	case "uniqueItems":
		if values, ok := value.([]Value); ok {
			for i := range values {
				for j := 0; j < i; j++ {
					if err := s.spend(check.Source, path, 1); err != nil {
						return err
					}
					same, err := s.equal(values[i], values[j], check.Source, path, 0)
					if err != nil {
						return err
					}
					if same {
						return bad()
					}
				}
			}
		}
	case "pattern":
		if text, ok := value.(string); ok {
			matched, err := validationPatternMatch(&check.Program, text, func(cost int) error { return s.spend(check.Source, path, cost) })
			if err != nil {
				return err
			}
			if !matched {
				return bad()
			}
		}
	default:
		return s.issue("evaluation_failure", check.Source, path, "unknown instruction: "+check.Op)
	}
	return nil
}
func (s *validationSession) equal(a, b Value, source ValidationSource, path string, depth int) (bool, error) {
	s.equality--
	if s.equality < 0 || depth > s.program.Limits.MaxDepth {
		return false, s.issue("evaluation_failure", source, path, "equality budget exhausted")
	}
	kind := validationKind(a)
	if kind != validationKind(b) {
		return false, nil
	}
	switch a := a.(type) {
	case Number, Integer:
		left, err := s.number(a, source, path)
		if err != nil {
			return false, err
		}
		right, err := s.number(b, source, path)
		if err != nil {
			return false, err
		}
		return left.compare(right) == 0, nil
	case []Value:
		right := b.([]Value)
		if len(a) != len(right) {
			return false, nil
		}
		for i := range a {
			same, err := s.equal(a[i], right[i], source, path, depth+1)
			if !same || err != nil {
				return same, err
			}
		}
		return true, nil
	case map[string]Value:
		right := b.(map[string]Value)
		if len(a) != len(right) {
			return false, nil
		}
		for key, value := range a {
			other, exists := right[key]
			if !exists {
				return false, nil
			}
			same, err := s.equal(value, other, source, path, depth+1)
			if !same || err != nil {
				return same, err
			}
		}
		return true, nil
	case nil:
		return true, nil
	case string:
		return a == b.(string), nil
	case bool:
		return a == b.(bool), nil
	default:
		return false, fmt.Errorf("invalid JSON representation")
	}
}
