//! The compiler's portable Thompson NFA, with caller-owned work accounting.
pub(super) struct PatternProgram {
    pub(super) start: usize,
    pub(super) states: &'static [PatternState],
}
#[allow(dead_code)]
pub(super) enum PatternState {
    Match,
    Char { ranges: &'static [[u32; 2]], target: usize },
    Split { first: usize, second: usize },
    Jump { target: usize },
    Start { target: usize },
    End { target: usize },
}
/// Evaluate with a caller-owned shared work budget.
pub(super) fn is_match(
    program: &PatternProgram,
    input: &str,
    remaining: &mut usize,
) -> Result<bool, ()> {
    fn charge(r: &mut usize) -> Result<(), ()> {
        if *r == 0 {
            Err(())
        } else {
            *r -= 1;
            Ok(())
        }
    }
    fn enqueue(
        index: usize,
        epoch: u32,
        seen: &mut [u32],
        stack: &mut Vec<usize>,
        remaining: &mut usize,
    ) -> Result<(), ()> {
        charge(remaining)?;
        if seen[index] != epoch {
            seen[index] = epoch;
            stack.push(index);
        }
        Ok(())
    }
    let mut seen = vec![0u32; program.states.len()];
    let mut stack = Vec::with_capacity(program.states.len());
    let mut active = Vec::with_capacity(program.states.len());
    let mut seeds = Vec::with_capacity(program.states.len());
    let mut next_seeds = Vec::with_capacity(program.states.len());
    let mut scalars = input.char_indices();
    let mut offset = 0usize;
    let mut epoch = 0u32;
    loop {
        charge(remaining)?;
        epoch = epoch.wrapping_add(1);
        if epoch == 0 {
            seen.fill(0);
            epoch = 1;
        }
        stack.clear();
        active.clear();
        enqueue(program.start, epoch, &mut seen, &mut stack, remaining)?;
        for seed in seeds.drain(..) {
            enqueue(seed, epoch, &mut seen, &mut stack, remaining)?;
        }
        while let Some(index) = stack.pop() {
            charge(remaining)?;
            match &program.states[index] {
                PatternState::Match => return Ok(true),
                PatternState::Split { first, second } => {
                    enqueue(*second, epoch, &mut seen, &mut stack, remaining)?;
                    enqueue(*first, epoch, &mut seen, &mut stack, remaining)?;
                }
                PatternState::Jump { target } => {
                    enqueue(*target, epoch, &mut seen, &mut stack, remaining)?
                }
                PatternState::Start { target } if offset == 0 => {
                    enqueue(*target, epoch, &mut seen, &mut stack, remaining)?
                }
                PatternState::End { target } if offset == input.len() => {
                    enqueue(*target, epoch, &mut seen, &mut stack, remaining)?
                }
                PatternState::Char { .. } => active.push(index),
                _ => {}
            }
        }
        let Some((byte_offset, scalar)) = scalars.next() else {
            return Ok(false);
        };
        debug_assert_eq!(byte_offset, offset);
        offset += scalar.len_utf8();
        next_seeds.clear();
        let scalar = scalar as u32;
        for index in active.drain(..) {
            if let PatternState::Char { ranges, target } = &program.states[index] {
                for range in *ranges {
                    charge(remaining)?;
                    if scalar < range[0] {
                        break;
                    }
                    if scalar <= range[1] {
                        next_seeds.push(*target);
                        break;
                    }
                }
            }
        }
        std::mem::swap(&mut seeds, &mut next_seeds);
    }
}