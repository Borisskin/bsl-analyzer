#!/usr/bin/env python3
"""Check the CI test wrappers preserve failures and reject empty test selections."""

from pathlib import Path
import re
import subprocess
import textwrap


workflow = Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml"
wrappers = re.findall(
    r"^          run_(?:exact|nonempty)\(\) \{\n.*?^          \}",
    workflow.read_text(),
    re.MULTILINE | re.DOTALL,
)
assert wrappers, "no CI test wrappers found"
for block in wrappers:
    name = block.strip().split("(", 1)[0]
    for passed, status in [(1, 0), (0, 0), (1, 101)]:
        output = f"test result: ok. {passed} passed; diagnostic marker"
        result = subprocess.run(
            ["bash", "-c", "set -euo pipefail\n"
             + f"cargo() {{ echo '{output}' >&2; return {status}; }}\n"
             + textwrap.dedent(block) + f"\n{name} fixture"],
            capture_output=True, text=True, check=False,
        )
        assert (result.returncode == 0) == (passed > 0 and status == 0), result
        assert output in result.stdout, result
print(f"PASS: {len(wrappers)} CI wrappers, success/empty/failure output")
