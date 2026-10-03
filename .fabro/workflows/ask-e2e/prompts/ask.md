Use the `fabro_ask` tool exactly once: ask run `{{ inputs.target_run }}` the
question "What did this run do? Summarize in one sentence."

Then reply with the analyst's answer verbatim, prefixed with `ANSWER: `.
If the tool call fails, reply with `ASK-FAILED: ` followed by the error
message.
