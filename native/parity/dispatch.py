# SPDX-License-Identifier: AGPL-3.0-only
"""Read the operation names out of the Python bridge's dispatcher.

desktop/winspace.py routes every request from the WebKit interface
through one ``dispatch`` method, which compares the request's ``method``
with string literals. bridge.json must list exactly those names, so this
module extracts them from the source without running it.

The extractor fails closed. Any use of the operation name that it cannot
read raises ValueError instead of being skipped, so an operation cannot
be added in a form that escapes the inventory.
"""
from __future__ import annotations

import ast
from typing import TypeAlias, TypeGuard

# The name the dispatcher gives the requested operation.
METHOD = 'method'

Parents: TypeAlias = dict[ast.AST, ast.AST]


def bridge_methods(source: str) -> set[str]:
    """Return every operation name that the dispatch function handles.

    Raise ValueError if the source does not have exactly one dispatch
    function, if that function names no operation, or if it uses the
    operation name in a way this module cannot read.
    """
    dispatcher = find_dispatcher(source)
    parents = parent_map(dispatcher)
    sources = method_sources(dispatcher)
    methods: set[str] = set()
    for node in ast.walk(dispatcher):
        check_method_use(node, parents, sources)
        methods |= named_operations(node)
    if not methods:
        raise ValueError('No legacy bridge operations were discovered.')
    return methods


def find_dispatcher(source: str) -> ast.FunctionDef:
    """Return the only function named ``dispatch`` in the source."""
    dispatchers = [
        node for node in ast.walk(ast.parse(source))
        if isinstance(node, ast.FunctionDef) and node.name == 'dispatch'
    ]
    if len(dispatchers) != 1:
        raise ValueError('Expected exactly one legacy dispatch function.')
    return dispatchers[0]


def parent_map(tree: ast.AST) -> Parents:
    """Map every node below ``tree`` to its parent node.

    The ast module records no parents, and deciding whether a use of
    ``method`` is safe depends on the expression around it.
    """
    parents: Parents = {}
    for node in ast.walk(tree):
        for child in ast.iter_child_nodes(node):
            parents[child] = node
    return parents


def named_operations(node: ast.AST) -> set[str]:
    """Return the operation names that one node dispatches on.

    Only a ``match method:`` statement and a comparison that has
    ``method`` as an operand name operations. Other nodes name none.
    """
    if isinstance(node, ast.Match) and is_method(node.subject):
        names: set[str] = set()
        for case in node.cases:
            names |= pattern_methods(case.pattern)
        return names
    if isinstance(node, ast.Compare) and compares_method(node):
        return string_values(dispatched_literals(node), node.lineno)
    return set()


def pattern_methods(pattern: ast.pattern) -> set[str]:
    """Return the operation names that a ``case`` pattern matches.

    A capture or wildcard such as ``case _:`` names none, and
    alternatives such as ``case 'a' | 'b':`` name all of their parts.
    Any other pattern raises ValueError, because it could match names
    that cannot be listed.
    """
    if isinstance(pattern, ast.MatchAs) and pattern.pattern is None:
        return set()
    if isinstance(pattern, ast.MatchOr):
        names: set[str] = set()
        for alternative in pattern.patterns:
            names |= pattern_methods(alternative)
        return names
    if isinstance(pattern, ast.MatchValue):
        name = string_literal(pattern.value)
        if name is not None:
            return {name}
    raise ValueError(
        f'Nonliteral bridge match pattern at line {pattern.lineno}.')


def dispatched_literals(node: ast.Compare) -> list[ast.expr]:
    """Return the expressions that a comparison with ``method`` selects.

    ``method == 'a'``, ``'a' == method`` and ``method in ('a', 'b')``
    select their literals. ``!=`` and ``not in`` are guards that exclude
    operations, so they select none. Any other shape raises ValueError,
    because what it selects cannot be known without running it.
    """
    if len(node.ops) != 1:
        raise ValueError(
            f'Chained bridge dispatch comparison at line {node.lineno}.')
    operator, left, right = node.ops[0], node.left, node.comparators[0]
    if isinstance(operator, (ast.NotEq, ast.NotIn)):
        return []
    if isinstance(operator, ast.Eq) and is_method(left):
        return [right]
    if isinstance(operator, ast.Eq) and is_method(right):
        return [left]
    if (isinstance(operator, ast.In) and is_method(left)
            and isinstance(right, (ast.Tuple, ast.List, ast.Set))):
        return right.elts
    raise ValueError(
        f'Unsupported bridge dispatch comparison at line {node.lineno}.')


def string_values(nodes: list[ast.expr], line: int) -> set[str]:
    """Return the values of string literals.

    Raise ValueError if any node is not a string literal: a variable or
    a computed value could name any operation.
    """
    values = set()
    for node in nodes:
        value = string_literal(node)
        if value is None:
            raise ValueError(f'Nonliteral bridge method at line {line}.')
        values.add(value)
    return values


def check_method_use(node: ast.AST, parents: Parents,
                     sources: list[ast.expr]) -> None:
    """Raise ValueError if a node reads the operation in an unknown way.

    A use this module cannot read, such as ``HANDLERS[method]``,
    ``getattr(self, 'on_' + method)``, ``request['method'] == 'x'`` or
    ``method.startswith('x')`` as a branch, could dispatch an operation
    that bridge.json never lists.
    """
    if is_method(node) and isinstance(node.ctx, ast.Load):
        understood = is_understood_name_use(node, parents)
        line = node.lineno
    elif is_method_lookup(node):
        understood = any(node is source for source in sources)
        line = node.lineno
    else:
        return
    if not understood:
        raise ValueError(
            f'Unsupported use of the bridge method name at line {line}.')


def is_understood_name_use(name: ast.Name, parents: Parents) -> bool:
    """Return whether a read of ``method`` is a form this module reads.

    Accepted forms are an operand of a comparison (which
    dispatched_literals then checks), the subject of ``match``, a
    ``method.startswith(...)`` guard that only raises, and an error
    message built with ``+``.
    """
    parent = parents.get(name)
    if isinstance(parent, ast.Compare):
        return True
    if isinstance(parent, ast.Match):
        return parent.subject is name
    if isinstance(parent, ast.Attribute):
        return is_prefix_guard(parent, parents)
    if isinstance(parent, ast.BinOp):
        return is_error_message(parent, parents)
    return False


def is_prefix_guard(attribute: ast.Attribute, parents: Parents) -> bool:
    """Return whether ``method.startswith('x')`` only guards a raise.

    The clipboard guard rejects a family of operations before dispatch
    and names none of them. The same call used as a branch that handles
    requests would dispatch operations that cannot be listed.
    """
    call = parents.get(attribute)
    if not isinstance(call, ast.Call):
        return False
    is_prefix_test = (
        attribute.attr == 'startswith'
        and call.func is attribute
        and len(call.args) == 1
        and not call.keywords
        and string_literal(call.args[0]) is not None
    )
    return is_prefix_test and guards_a_raise(call, parents)


def guards_a_raise(expression: ast.expr, parents: Parents) -> bool:
    """Return whether an expression is in a raise-only ``if`` test.

    The expression may be combined with others through ``and``, ``or``
    and ``not``. The ``if`` must have no ``else`` or ``elif``, whose
    body would run for the operations the guard lets through.
    """
    test: ast.AST = expression
    while is_logical(parents.get(test)):
        test = parents[test]
    statement = parents.get(test)
    return (
        isinstance(statement, ast.If)
        and statement.test is test
        and not statement.orelse
        and all(isinstance(line, ast.Raise) for line in statement.body)
    )


def is_logical(node: ast.AST | None) -> bool:
    """Return whether a node combines conditions with and, or or not."""
    if isinstance(node, ast.UnaryOp):
        return isinstance(node.op, ast.Not)
    return isinstance(node, ast.BoolOp)


def is_error_message(expression: ast.BinOp, parents: Parents) -> bool:
    """Return whether ``+`` only builds a raised exception's message.

    This accepts ``raise ValueError('Unknown action: ' + method)``.
    Other string building is refused, because
    ``getattr(self, 'on_' + method)`` is built the same way and it
    dispatches.
    """
    call = parents.get(expression)
    if not (isinstance(expression.op, ast.Add)
            and isinstance(call, ast.Call)
            and any(argument is expression for argument in call.args)):
        return False
    statement = parents.get(call)
    return isinstance(statement, ast.Raise) and statement.exc is call


def method_sources(dispatcher: ast.FunctionDef) -> list[ast.expr]:
    """Return the values that top-level assignments store in method.

    The dispatcher reads the operation from the request once, as in
    ``method, a = request['method'], request['args']``. Any other read
    of ``request['method']`` would bypass the name this module follows.
    """
    sources = []
    for statement in dispatcher.body:
        if isinstance(statement, ast.Assign) and len(statement.targets) == 1:
            pairs = assigned_pairs(statement.targets[0], statement.value)
            sources += [value for target, value in pairs if is_method(target)]
    return sources


def assigned_pairs(target: ast.expr,
                   value: ast.expr) -> list[tuple[ast.expr, ast.expr]]:
    """Pair each target of an assignment with its value.

    ``a, b = x, y`` gives ``[(a, x), (b, y)]``, and ``a = x`` gives
    ``[(a, x)]``.
    """
    if (isinstance(target, ast.Tuple) and isinstance(value, ast.Tuple)
            and len(target.elts) == len(value.elts)):
        return list(zip(target.elts, value.elts))
    return [(target, value)]


def compares_method(node: ast.Compare) -> bool:
    """Return whether ``method`` is an operand of a comparison."""
    operands = [node.left, *node.comparators]
    return any(is_method(operand) for operand in operands)


def is_method(node: ast.AST) -> TypeGuard[ast.Name]:
    """Return whether an expression is the name ``method``."""
    return isinstance(node, ast.Name) and node.id == METHOD


def is_method_lookup(node: ast.AST) -> TypeGuard[ast.Subscript]:
    """Return whether an expression reads ``request['method']``."""
    return (
        isinstance(node, ast.Subscript)
        and isinstance(node.ctx, ast.Load)
        and isinstance(node.slice, ast.Constant)
        and node.slice.value == METHOD
    )


def string_literal(node: ast.AST) -> str | None:
    """Return the text of a string literal, or None for other nodes."""
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return None
