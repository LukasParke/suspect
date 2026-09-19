// Checked, immutable v3 resource metadata. No URI resolution or acquisition runs
// during validation: dynamic selection uses only these indexed bindings.
package sdk

import (
	"encoding/json"
	"errors"
	"net/url"
	"strings"
	"unicode"
)

type validationResourceContext struct {
	Resources  []validationResource
	NodeScopes []validationNodeScope
}
type validationResource struct {
	Source                      ValidationSource
	Kind, CanonicalURI, BaseURI string
	Aliases                     []string
	DeclarationSource           *ValidationSource
	DynamicAnchors              []validationDynamicBinding
}
type validationDynamicBinding struct {
	Name   string
	Source ValidationSource
	Target int
}

func (b *validationDynamicBinding) UnmarshalJSON(data []byte) error {
	var tuple []json.RawMessage
	if err := json.Unmarshal(data, &tuple); err != nil {
		return err
	}
	if len(tuple) != 3 {
		return errors.New("invalid dynamic anchor tuple")
	}
	if err := json.Unmarshal(tuple[0], &b.Name); err != nil {
		return err
	}
	if err := json.Unmarshal(tuple[1], &b.Source); err != nil {
		return err
	}
	return json.Unmarshal(tuple[2], &b.Target)
}

type validationNodeScope struct {
	Resource   int
	SchemaRoot ValidationSource
	Address    string
}

func (s *validationNodeScope) UnmarshalJSON(data []byte) error {
	var tuple []json.RawMessage
	if err := json.Unmarshal(data, &tuple); err != nil {
		return err
	}
	if len(tuple) != 3 {
		return errors.New("invalid node scope tuple")
	}
	if err := json.Unmarshal(tuple[0], &s.Resource); err != nil {
		return err
	}
	if err := json.Unmarshal(tuple[1], &s.SchemaRoot); err != nil {
		return err
	}
	return json.Unmarshal(tuple[2], &s.Address)
}
func validationSourceContains(parent, child ValidationSource) bool {
	return parent.Document == child.Document && (parent.Pointer == child.Pointer || strings.HasPrefix(child.Pointer, parent.Pointer+"/"))
}
func validationResourceURI(value string) bool {
	u, err := url.Parse(value)
	return err == nil && u.IsAbs() && strings.IndexFunc(value, func(r rune) bool { return unicode.IsControl(r) || unicode.IsSpace(r) }) < 0
}
func checkValidationResources(program *validationProgram) {
	context := program.ResourceContext
	if context == nil || len(context.NodeScopes) != len(program.Nodes) {
		panic("invalid v3 node/resource alignment")
	}
	aliases := map[string]int{}
	for index, resource := range context.Resources {
		if !validationResourceURI(resource.CanonicalURI) || !validationResourceURI(resource.BaseURI) || strings.Contains(resource.BaseURI, "#") || len(resource.Aliases) == 0 {
			panic("invalid resource URI metadata")
		}
		if resource.Kind != "document" && resource.Kind != "openApiDocument" && resource.Kind != "schema" {
			panic("unknown resource kind")
		}
		for _, alias := range resource.Aliases {
			if !validationResourceURI(alias) {
				panic("invalid resource alias")
			}
			if owner, found := aliases[alias]; found && owner != index {
				panic("ambiguous resource alias")
			}
			aliases[alias] = index
		}
		names := map[string]bool{}
		for _, binding := range resource.DynamicAnchors {
			if binding.Name == "" || names[binding.Name] || binding.Target < 0 || binding.Target >= len(program.Nodes) || context.NodeScopes[binding.Target].Resource != index || !validationSourceContains(resource.Source, binding.Source) {
				panic("invalid indexed dynamic binding")
			}
			names[binding.Name] = true
		}
	}
	for index, scope := range context.NodeScopes {
		if scope.Resource < 0 || scope.Resource >= len(context.Resources) || !validationSourceContains(context.Resources[scope.Resource].Source, program.Nodes[index].Source) || !validationSourceContains(scope.SchemaRoot, program.Nodes[index].Source) || !validationResourceURI(scope.Address) {
			panic("invalid indexed node resource scope")
		}
		for _, check := range program.Nodes[index].Checks {
			if check.Op == "dynamicRef" {
				if check.target < 0 || check.target >= len(program.Nodes) || check.InitialResource < 0 || check.InitialResource >= len(context.Resources) || context.NodeScopes[check.target].Resource != check.InitialResource {
					panic("invalid dynamic initial target/resource")
				}
				if check.Anchor != nil {
					found := false
					for _, binding := range context.Resources[check.InitialResource].DynamicAnchors {
						if binding.Name == *check.Anchor && binding.Target == check.target {
							found = true
						}
					}
					if !found {
						panic("dynamic name is not the indexed initial binding")
					}
				}
			}
		}
	}
}

type validationContextEdge struct{ previous, resource int }
type validationResources struct {
	order    []int
	entered  map[int]bool
	context  int
	contexts map[validationContextEdge]int
}

func newValidationResources() *validationResources {
	return &validationResources{entered: map[int]bool{}, contexts: map[validationContextEdge]int{}}
}
func (v *validationScoped) enterResource(index int, source ValidationSource, path string) (func(), error) {
	if v.resources == nil {
		return func() {}, nil
	}
	resource := v.session.program.ResourceContext.NodeScopes[index].Resource
	if v.resources.entered[resource] {
		return func() {}, nil
	}
	if err := v.step(source, path); err != nil {
		return nil, err
	}
	state := v.resources
	previous := state.context
	edge := validationContextEdge{previous, resource}
	next, exists := state.contexts[edge]
	if !exists {
		next = len(state.contexts) + 1
		state.contexts[edge] = next
	}
	state.context = next
	state.entered[resource] = true
	state.order = append(state.order, resource)
	return func() {
		delete(state.entered, resource)
		state.order = state.order[:len(state.order)-1]
		state.context = previous
	}, nil
}
func (v *validationScoped) dynamicTarget(check validationCheck, path string) (int, error) {
	if v.resources == nil {
		return 0, v.session.issue("evaluation_failure", check.Source, path, "dynamic reference has no v3 context")
	}
	if check.Anchor == nil {
		return check.target, nil
	}
	for _, resource := range v.resources.order {
		if err := v.step(check.Source, path); err != nil {
			return 0, err
		}
		for _, binding := range v.session.program.ResourceContext.Resources[resource].DynamicAnchors {
			if err := v.step(check.Source, path); err != nil {
				return 0, err
			}
			if binding.Name == *check.Anchor {
				return binding.Target, nil
			}
		}
	}
	return check.target, nil
}
