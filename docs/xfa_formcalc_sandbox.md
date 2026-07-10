# FormCalc sandbox

FormCalc execution is off by default. `formcalc-safe-subset` plus explicit event execution enables pure calculate/validate expressions only.

The evaluator supports finite numeric arithmetic, `&`/`Concat` string concatenation, comparison and Boolean operators, `if/then/else/endif`, bounded field reads, and pure helpers such as Sum, Avg, Min, Max, Abs, Round, Floor, Ceil, Len, Upper, and Lower. XML field strings are numerically coerced for arithmetic.

It blocks external data functions, host calls, network, files, process/shell, native code, environment access, dynamic evaluation, arbitrary DOM resolution, loops, and recursion bombs. Defaults include 10,000 instructions, 256 KiB source, 8 MiB estimated script memory, call depth 32, 1 MiB strings, 1,024 properties/arguments and 1,024 field mutations. Audit logs contain hashes/targets/outcomes, never script source or secret values.
