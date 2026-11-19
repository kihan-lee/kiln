import ast
import sys
import tokenize

path = sys.argv[1]
try:
    with open(path, "rb") as f:
        for token in tokenize.tokenize(f.readline):
            if token.type == tokenize.COMMENT:
                print(token.start[0], token.string, sep="\t")
    with open(path, "rb") as f:
        body = ast.parse(f.read()).body
    first = body[0] if body else None
    if isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant) and isinstance(first.value.value, str):
        lines = first.value.value.strip().splitlines()
        print(first.lineno, "파일 맨 위 설명문: " + (lines[0][:60] if lines else ""), sep="\t")
except (tokenize.TokenError, SyntaxError, ValueError):
    pass
