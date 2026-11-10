# `uniqueItems` compares only items that share a fingerprint

An array longer than 128 items is grouped by a hash that equal values share (a scalar's exact value; a container's length and its members one level deep), and only items in the same group are compared. Reading a fingerprint spends the same equality allowance as a comparison, so a limit too small to decide is still an evaluation failure, never "unique". An item whose fingerprint cannot be read (a number over the operand limit, undecodable text) sends the whole array back to pairwise comparison, which reports that failure as before.

## Considered Options

- Compare every pair, as the validator first did: rejected. A list of 3,000 ids needs 4.5 million comparisons, past any budget sized for a response, so a conforming reply was reported as a violation.
- Keep the pairs and raise the contract runtime's budget with the square of the document: rejected. It moves the cliff instead of removing it (a list of 50,000 ids fails again) and a second of work per reply becomes ordinary.

## Consequences

Arrays of up to 128 items behave exactly as before, including which resource failure is reported. Many items that share a fingerprint without being equal (records that differ only two levels down) are still compared pair by pair within their group.
