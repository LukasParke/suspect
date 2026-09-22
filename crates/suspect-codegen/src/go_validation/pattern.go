package sdk

type validationPattern struct {
	Version string
	Start   int
	States  []validationPatternState
}
type validationPatternState struct {
	Op                    string
	Target, First, Second int
	Ranges                [][2]rune
}

func validationPatternMatch(program *validationPattern, text string, spend func(int) error) (bool, error) {
	chars := []rune(text)
	for offset := 0; offset <= len(chars); offset++ {
		current := map[int]bool{program.Start: true}
		for position := offset; position <= len(chars); position++ {
			pending := make([]int, 0, len(current))
			for id := range current {
				pending = append(pending, id)
			}
			seen := map[int]bool{}
			consuming := []int{}
			for len(pending) > 0 {
				if err := spend(1); err != nil {
					return false, err
				}
				id := pending[len(pending)-1]
				pending = pending[:len(pending)-1]
				if seen[id] {
					continue
				}
				seen[id] = true
				state := program.States[id]
				switch state.Op {
				case "match":
					return true, nil
				case "split":
					pending = append(pending, state.First, state.Second)
				case "jump":
					pending = append(pending, state.Target)
				case "start":
					if position == 0 {
						pending = append(pending, state.Target)
					}
				case "end":
					if position == len(chars) {
						pending = append(pending, state.Target)
					}
				case "char":
					consuming = append(consuming, id)
				}
			}
			if position == len(chars) {
				break
			}
			current = map[int]bool{}
			for _, id := range consuming {
				state := program.States[id]
				for _, interval := range state.Ranges {
					if err := spend(1); err != nil {
						return false, err
					}
					if interval[0] <= chars[position] && chars[position] <= interval[1] {
						current[state.Target] = true
						break
					}
				}
			}
			if len(current) == 0 {
				break
			}
		}
	}
	return false, nil
}
