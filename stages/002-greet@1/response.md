Hello, and demo complete! 👋

**What happened:**
- **Say hello** — hi there! 👋
- **Demonstrated the basic Fabro workflow** — this task is the `greet` step of `.fabro/workflows/hello` (`start → greet → exit`); your prompt matched its goal/prompt verbatim, so I executed it as that step.
- **The edit** — created `README.md` at the repo root (it didn't exist) containing a short 5-7-5 haiku:

> New run greets the world —
> branches bloom, the diff blooms green,
> haiku marks hello.

**Constraints honored:** no builds, no tests, no project tooling — the only commands run were file reads and a plain `git status` to verify the change. Working tree shows just the new `README.md`; I've left committing to the Fabro run machinery as usual.