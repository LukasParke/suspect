package sdk

// Exact v2 NFA visit schedule: every position, enqueue (including duplicates),
// stack pop, and tested character interval consumes the caller's shared budget.
func validationScopedPattern(program *validationPattern, text string, step func() error) (bool, error) {
	chars := []rune(text)
	seeds := []int{}
	for offset := 0; offset <= len(chars); offset++ {
		if err := step(); err != nil {
			return false, err
		}
		seen := make([]bool, len(program.States))
		stack := []int{}
		active := []int{}
		enqueue := func(index int) error {
			if err := step(); err != nil {
				return err
			}
			if !seen[index] {
				seen[index] = true
				stack = append(stack, index)
			}
			return nil
		}
		if err := enqueue(program.Start); err != nil {
			return false, err
		}
		for _, seed := range seeds {
			if err := enqueue(seed); err != nil {
				return false, err
			}
		}
		for len(stack) > 0 {
			if err := step(); err != nil {
				return false, err
			}
			index := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			state := program.States[index]
			switch state.Op {
			case "match":
				return true, nil
			case "split":
				if err := enqueue(state.Second); err != nil {
					return false, err
				}
				if err := enqueue(state.First); err != nil {
					return false, err
				}
			case "jump":
				if err := enqueue(state.Target); err != nil {
					return false, err
				}
			case "start":
				if offset == 0 {
					if err := enqueue(state.Target); err != nil {
						return false, err
					}
				}
			case "end":
				if offset == len(chars) {
					if err := enqueue(state.Target); err != nil {
						return false, err
					}
				}
			case "char":
				active = append(active, index)
			}
		}
		if offset == len(chars) {
			return false, nil
		}
		seeds = nil
		for _, index := range active {
			state := program.States[index]
			for _, interval := range state.Ranges {
				if err := step(); err != nil {
					return false, err
				}
				if chars[offset] < interval[0] {
					break
				}
				if chars[offset] <= interval[1] {
					seeds = append(seeds, state.Target)
					break
				}
			}
		}
	}
	return false, nil
}
