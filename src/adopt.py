import ast
import json
import sys
import warnings

warnings.simplefilter("ignore", SyntaxWarning)


def docstring(tree):
    first = tree.body[0] if tree.body else None
    if (
        isinstance(first, ast.Expr)
        and isinstance(first.value, ast.Constant)
        and isinstance(first.value.value, str)
    ):
        return first
    return None


def shape(source):
    tree = ast.parse(source)
    if docstring(tree):
        tree.body = tree.body[1:]
    return ast.dump(tree)


def drop_docstring(source):
    tree = ast.parse(source)
    node = docstring(tree)
    if node is None:
        return source, None
    if ">>>" in node.value.value:
        return source, "doctest가 있어 설명문 유지"
    if any(isinstance(n, ast.Name) and n.id == "__doc__" for n in ast.walk(tree)):
        return source, "__doc__를 써서 설명문 유지"
    lines = source.splitlines(keepends=True)
    start, end = node.lineno - 1, node.end_lineno - 1
    before = lines[start].encode()[: node.col_offset].decode(errors="replace")
    after = lines[end].encode()[node.end_col_offset :].decode()
    if before.strip() or after.strip():
        return source, "같은 줄에 코드가 있어 설명문 유지"
    rest = lines[end + 1 :]
    while rest and not rest[0].strip():
        rest = rest[1:]
    return "".join(lines[:start] + rest), "removed"


def failure(error):
    reason = error.msg if isinstance(error, SyntaxError) else error
    return {"ok": False, "error": f"구문 분석 실패: {reason}"}


def clean(item):
    try:
        original = shape(item["before"])
        text, docstring_result = drop_docstring(item["after"])
        same = shape(text) == original
    except (SyntaxError, ValueError) as error:
        return failure(error)
    if not same:
        return {"ok": False, "error": "지운 뒤 코드가 달라짐"}
    return {"ok": True, "text": text, "docstring": docstring_result}


def same(item):
    try:
        return {"ok": shape(item["before"]) == shape(item["after"])}
    except (SyntaxError, ValueError) as error:
        return failure(error)


def main():
    request = json.load(sys.stdin)
    run = clean if request["mode"] == "clean" else same
    json.dump([run(item) for item in request["items"]], sys.stdout)


main()
