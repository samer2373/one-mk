#!/usr/bin/env python3
"""Tests the PATH and start-at-sign-in logic of the Windows installer.

    python3 scripts/test-windows-path.py

installer/one-mk.iss cannot be compiled or run here (Inno Setup is Windows
only; CI compiles and installs it on a Windows runner). Its pure string
functions (PathKey, PathHasEntry, PathWithEntry, PathWithoutEntry,
RunValueProgram, RunValueIsOurs) use a small, plain part of Pascal, so this
file reads them out of the .iss and runs that very text in a tiny interpreter
of exactly that subset (strings, if/while/begin..end, Pos, Copy, Length, Trim,
Lowercase, StringChangeEx; booleans are not short-circuited, to be safe). The
results are compared with an independent model written the plain way in
Python, on fixed cases (including the ones that corrupt a PATH when done
naively: a longer entry that starts with the install folder) and on random
ones. It also checks the order of the uninstall steps. What this cannot show:
that Inno Setup accepts the script and that the registry calls behave on a
real Windows computer.
"""
import random
import re
import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ISS = REPO / "installer" / "one-mk.iss"
PURE = ["PathKey", "PathHasEntry", "PathWithEntry", "PathWithoutEntry", "RunValueProgram", "RunValueIsOurs"]


# ----------------------------------------------------- Pascal subset runner
TOKEN = re.compile(r"""\s+|\{[^}]*\}|//[^\n]*|(?P<str>'(?:[^']|'')*')|(?P<num>\d+)|(?P<id>[A-Za-z_]\w*)|(?P<sym>:=|<>|<=|>=|[()=<>,;:+\-*.])""")


def tokenize(text):
    tokens, pos = [], 0
    while pos < len(text):
        match = TOKEN.match(text, pos)
        if not match:
            raise SyntaxError(f"cannot read {text[pos:pos + 30]!r}")
        pos = match.end()
        if match.group("str") is not None:
            tokens.append(("str", match.group("str")[1:-1].replace("''", "'")))
        elif match.group("num") is not None:
            tokens.append(("num", int(match.group("num"))))
        elif match.group("id") is not None:
            tokens.append(("id", match.group("id")))
        elif match.group("sym") is not None:
            tokens.append(("sym", match.group("sym")))
    return tokens


class Exit(Exception):
    pass


class Function:
    """One `function Name(A, B: string): Type; var ...; begin ... end;`."""

    def __init__(self, source):
        self.t = tokenize(source)
        self.i = 0
        self.expect_id("function")
        self.name = self.take("id")
        self.params = []
        self.expect_sym("(")
        while not self.is_sym(")"):
            names = [self.take("id")]
            while self.is_sym(","):
                self.i += 1
                names.append(self.take("id"))
            self.expect_sym(":")
            self.take("id")
            self.params += names
            if self.is_sym(";"):
                self.i += 1
        self.expect_sym(")")
        self.expect_sym(":")
        self.result_type = self.take("id")
        self.expect_sym(";")
        self.locals = {}
        if self.is_id("var"):
            self.i += 1
            while not self.is_id("begin"):
                names = [self.take("id")]
                while self.is_sym(","):
                    self.i += 1
                    names.append(self.take("id"))
                self.expect_sym(":")
                kind = self.take("id").lower()
                self.expect_sym(";")
                for name in names:
                    self.locals[name.lower()] = {"integer": 0, "boolean": False}.get(kind, "")
        self.body = self.block()
        self.expect_sym(";")
        assert self.i == len(self.t), f"{self.name}: text after the function"

    # -- token helpers
    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else ("end", None)

    def take(self, kind):
        token = self.peek()
        assert token[0] == kind, f"{self.name}: expected {kind}, got {token}"
        self.i += 1
        return token[1]

    def is_sym(self, s):
        return self.peek() == ("sym", s)

    def is_id(self, s):
        token = self.peek()
        return token[0] == "id" and token[1].lower() == s

    def expect_sym(self, s):
        assert self.is_sym(s), f"{self.name}: expected {s!r}, got {self.peek()}"
        self.i += 1

    def expect_id(self, s):
        assert self.is_id(s), f"{self.name}: expected {s!r}, got {self.peek()}"
        self.i += 1

    # -- statements (as nested tuples)
    def block(self):
        self.expect_id("begin")
        body = []
        while not self.is_id("end"):
            body.append(self.statement())
            if self.is_sym(";"):
                self.i += 1
        self.expect_id("end")
        return ("block", body)

    def statement(self):
        if self.is_id("begin"):
            return self.block()
        if self.is_id("if"):
            self.i += 1
            condition = self.expression()
            self.expect_id("then")
            then = self.statement()
            other = None
            if self.is_id("else"):
                self.i += 1
                other = self.statement()
            return ("if", condition, then, other)
        if self.is_id("while"):
            self.i += 1
            condition = self.expression()
            self.expect_id("do")
            return ("while", condition, self.statement())
        if self.is_id("exit"):
            self.i += 1
            return ("exit",)
        name = self.take("id")
        if self.is_sym(":="):
            self.i += 1
            return ("assign", name.lower(), self.expression())
        assert name.lower() == "stringchangeex", f"{self.name}: unexpected call {name}"
        self.expect_sym("(")
        target = self.take("id").lower()
        args = []
        while self.is_sym(","):
            self.i += 1
            args.append(self.expression())
        self.expect_sym(")")
        return ("change", target, args)

    # -- expressions, with Pascal's precedence (relational lowest)
    def expression(self):
        left = self.simple()
        if self.peek()[0] == "sym" and self.peek()[1] in ("=", "<>", "<", ">", "<=", ">="):
            op = self.take("sym")
            return ("bin", op, left, self.simple())
        return left

    def simple(self):
        left = self.term()
        while (self.peek()[0] == "sym" and self.peek()[1] in ("+", "-")) or self.is_id("or"):
            op = "or" if self.is_id("or") else self.peek()[1]
            self.i += 1
            left = ("bin", op, left, self.term())
        return left

    def term(self):
        left = self.factor()
        while self.is_id("and") or self.is_sym("*"):
            op = "and" if self.is_id("and") else "*"
            self.i += 1
            left = ("bin", op, left, self.factor())
        return left

    def factor(self):
        kind, value = self.peek()
        if kind in ("str", "num"):
            self.i += 1
            return ("lit", value)
        if self.is_id("not"):
            self.i += 1
            return ("not", self.factor())
        if self.is_sym("("):
            self.i += 1
            inner = self.expression()
            self.expect_sym(")")
            return inner
        name = self.take("id")
        low = name.lower()
        if low in ("true", "false"):
            return ("lit", low == "true")
        if self.is_sym("("):
            self.i += 1
            args = []
            while not self.is_sym(")"):
                args.append(self.expression())
                if self.is_sym(","):
                    self.i += 1
            self.expect_sym(")")
            return ("call", low, args)
        return ("var", low)

    # -- running
    def __call__(self, *args):
        self.env = {"result": {"boolean": False, "integer": 0}.get(self.result_type.lower(), "")}
        self.env.update(self.locals)
        assert len(args) == len(self.params), self.name
        for name, value in zip(self.params, args):
            self.env[name.lower()] = value
        try:
            self.run(self.body)
        except Exit:
            pass
        return self.env["result"]

    def run(self, node):
        kind = node[0]
        if kind == "block":
            for statement in node[1]:
                self.run(statement)
        elif kind == "assign":
            assert node[1] in self.env, f"{self.name}: assignment to unknown {node[1]}"
            self.env[node[1]] = self.value(node[2])
        elif kind == "if":
            if self.value(node[1]):
                self.run(node[2])
            elif node[3] is not None:
                self.run(node[3])
        elif kind == "while":
            guard = 0
            while self.value(node[1]):
                self.run(node[2])
                guard += 1
                assert guard < 10000, f"{self.name}: loop does not end"
        elif kind == "exit":
            raise Exit
        elif kind == "change":
            text, old, new = self.env[node[1]], self.value(node[2][0]), self.value(node[2][1])
            self.env[node[1]] = text.replace(old, new) if old else text
        else:
            raise AssertionError(kind)

    def value(self, node):
        kind = node[0]
        if kind == "lit":
            return node[1]
        if kind == "var":
            return self.env[node[1]]
        if kind == "not":
            return not self.value(node[1])
        if kind == "bin":
            op = node[1]
            a, b = self.value(node[2]), self.value(node[3])  # both always evaluated
            return {"+": lambda: a + b, "-": lambda: a - b, "*": lambda: a * b, "=": lambda: a == b, "<>": lambda: a != b,
                    "<": lambda: a < b, ">": lambda: a > b, "<=": lambda: a <= b, ">=": lambda: a >= b,
                    "and": lambda: a and b, "or": lambda: a or b}[op]()
        name, args = node[1], [self.value(a) for a in node[2]]
        if name == "pos":
            return args[1].find(args[0]) + 1
        if name == "copy":
            text, start, count = args  # Pascal: from 1; a start below 1 or past the end gives ''
            return text[start - 1:start - 1 + count] if start >= 1 and count > 0 else ""
        if name == "length":
            return len(args[0])
        if name == "trim":
            return args[0].strip(" \t\r\n")
        if name == "lowercase":
            return args[0].lower()
        raise AssertionError(f"{self.name}: unknown function {name}")


def pascal():
    text = ISS.read_text(encoding="utf-8")
    functions = {}
    for name in PURE:
        match = re.search(rf"^function {name}\(.*?^end;\n", text, re.M | re.S)
        assert match, f"installer/one-mk.iss has no function {name}"
        functions[name] = Function(match.group(0))
    return functions


def link(functions):
    """Let the functions call each other (PathKey inside PathHasEntry, ...)."""
    original = Function.value

    def value(self, node):
        if node[0] == "call" and node[1] in {n.lower() for n in functions}:
            callee = {n.lower(): f for n, f in functions.items()}[node[1]]
            return callee(*[self.value(a) for a in node[2]])
        return original(self, node)

    Function.value = value


FUNCTIONS = pascal()
link(FUNCTIONS)


# --------------------------------------------- the same thing, the plain way
def key(entry):
    entry = entry.strip(" \t\r\n").lower()
    while len(entry) > 3 and entry.endswith("\\"):
        entry = entry[:-1]
    return entry


def model_has(entries, folder):
    return any(e != "" and key(e) == key(folder) for e in entries.split(";"))


def model_with(entries, folder):
    if model_has(entries, folder):
        return entries
    if entries.strip() == "":
        return folder
    return entries + folder if entries.endswith(";") else entries + ";" + folder


def model_without(entries, folder):
    return ";".join(e for e in entries.split(";") if e == "" or key(e) != key(folder))


def model_program(data):
    data = data.strip()
    if data.startswith('"'):
        end = data.find('"', 1)
        return "" if end < 0 else data[1:end]
    return data.split(" ")[0]


def model_ours(data, folder):
    program = model_program(data).lower().replace("/", "\\")
    if "\\" not in program:
        return False
    return key(program.rsplit("\\", 1)[0]) == key(folder)


APP = r"C:\Users\Jo\AppData\Local\Programs\one-mk"


class PathLogic(unittest.TestCase):
    def has(self, entries, folder=APP):
        return FUNCTIONS["PathHasEntry"](entries, folder)

    def add(self, entries, folder=APP):
        return FUNCTIONS["PathWithEntry"](entries, folder)

    def remove(self, entries, folder=APP):
        return FUNCTIONS["PathWithoutEntry"](entries, folder)

    def test_a_longer_entry_that_starts_with_the_folder_is_not_ours(self):
        longer = APP + r"-tools"
        self.assertFalse(self.has(longer))
        self.assertEqual(self.remove(rf"C:\Windows;{longer};{APP};C:\Other"), rf"C:\Windows;{longer};C:\Other")
        self.assertEqual(self.add(longer), f"{longer};{APP}")

    def test_a_sub_folder_and_a_parent_are_not_ours(self):
        self.assertFalse(self.has(APP + r"\bin"))
        self.assertFalse(self.has(r"C:\Users\Jo\AppData\Local\Programs"))

    def test_case_and_a_trailing_backslash_do_not_matter(self):
        for spelling in (APP.upper(), APP.lower(), APP + "\\", APP + "\\\\"):
            self.assertTrue(self.has(rf"C:\a;{spelling};C:\b"), spelling)
            self.assertEqual(self.add(rf"C:\a;{spelling}"), rf"C:\a;{spelling}", "an upgrade never adds a second entry")
            self.assertEqual(self.remove(rf"C:\a;{spelling};C:\b"), r"C:\a;C:\b")

    def test_running_the_installer_twice_adds_the_entry_once(self):
        once = self.add(r"C:\Windows;C:\Other")
        self.assertEqual(once, rf"C:\Windows;C:\Other;{APP}")
        self.assertEqual(self.add(once), once)

    def test_adding_and_removing_leaves_the_path_as_it_was(self):
        for original in ("", r"C:\Windows", r"C:\Windows;C:\Other", r"C:\a;;C:\b;", r"%SystemRoot%\system32;%USERPROFILE%\bin"):
            self.assertEqual(self.remove(self.add(original)), original.rstrip(";") if original.endswith(";") and original else original, original)

    def test_other_entries_are_kept_exactly_empty_ones_and_variables_too(self):
        self.assertEqual(self.remove(rf"%USERPROFILE%\bin;;{APP};C:\b;"), r"%USERPROFILE%\bin;;C:\b;")
        self.assertEqual(self.remove(rf"{APP};C:\a;{APP}"), r"C:\a")
        self.assertEqual(self.remove(APP), "")
        self.assertEqual(self.remove(""), "")
        self.assertEqual(self.remove(r"C:\a;C:\b"), r"C:\a;C:\b", "nothing to remove: unchanged, so the installer does not rewrite it")

    def test_a_long_path_is_not_cut(self):
        entries = ";".join(rf"C:\Some\Long\Folder\Name\{n}" for n in range(300))  # far past 1024 characters
        self.assertGreater(len(entries), 8000)
        self.assertEqual(self.remove(self.add(entries)), entries)
        self.assertTrue(self.add(entries).endswith(";" + APP))

    def test_the_root_of_a_drive_keeps_its_backslash(self):
        self.assertEqual(FUNCTIONS["PathKey"]("C:\\"), "c:\\")
        self.assertEqual(FUNCTIONS["PathKey"]("  C:\\Foo\\  "), "c:\\foo")

    def test_the_run_value_is_ours_only_if_the_program_sits_in_the_folder(self):
        ours = FUNCTIONS["RunValueIsOurs"]
        yes = [rf'"{APP}\one-mk-tray.exe" --start-daemon', rf"{APP}\one-mk.exe run", rf'"{APP.lower()}\ONE-MK.EXE" run',
               rf'"{APP}\one-mk-tray.exe"', APP.replace("\\", "/") + "/one-mk.exe run"]
        no = [r'"C:\Program Files\other\x.exe"', rf'"{APP}-tools\x.exe"', rf'"{APP}\bin\x.exe"', "", '"', "one-mk.exe run",
              rf"{APP}\sub\one-mk.exe"]
        for value in yes:
            self.assertTrue(ours(value, APP), value)
        for value in no:
            self.assertFalse(ours(value, APP), value)
        # A folder with a space in it is quoted in the value, as `one-mk install` writes it.
        folder = r"C:\Users\Jo Smith\AppData\Local\Programs\one-mk"
        self.assertTrue(ours(rf'"{folder}\one-mk-tray.exe" --start-daemon', folder))
        self.assertFalse(ours(rf'"{folder}\one-mk-tray.exe" --start-daemon', APP))

    def test_the_pascal_text_agrees_with_the_plain_model_on_random_inputs(self):
        rng = random.Random(1)
        pieces = [APP, APP.upper(), APP + "\\", APP + "-tools", APP + r"\bin", r"C:\Windows", r"%SystemRoot%\system32",
                  "", " ", r"C:\a b\c", "C:\\", r"D:\x\\"]
        for _ in range(3000):
            entries = ";".join(rng.choice(pieces) for _ in range(rng.randint(0, 6)))
            folder = rng.choice([APP, APP + "\\", APP.lower(), r"C:\Windows", r"D:\x"])
            self.assertEqual(self.has(entries, folder), model_has(entries, folder), (entries, folder))
            self.assertEqual(self.add(entries, folder), model_with(entries, folder), (entries, folder))
            self.assertEqual(self.remove(entries, folder), model_without(entries, folder), (entries, folder))
            self.assertFalse(model_has(self.remove(entries, folder), folder), "no entry left")
            self.assertTrue(model_has(self.add(entries, folder), folder), "the entry is there")
            self.assertEqual(self.add(self.add(entries, folder), folder), self.add(entries, folder), "adding twice")
            data = rng.choice([f'"{folder}\\one-mk-tray.exe" --start-daemon', f"{folder}\\one-mk.exe run", f'"{folder}-x\\a.exe"',
                               "", '"', rf"C:\a b\x.exe y", f"{folder.replace(chr(92), '/')}/one-mk.exe run", rf'"{folder}\sub\a.exe"'])
            self.assertEqual(FUNCTIONS["RunValueIsOurs"](data, folder), model_ours(data, folder), (data, folder))


class UninstallSteps(unittest.TestCase):
    """The order of the uninstall steps, read from the script text."""

    def setUp(self):
        text = ISS.read_text(encoding="utf-8")
        match = re.search(r"^procedure CurUninstallStepChanged\(.*?^end;\n", text, re.M | re.S)
        self.assertIsNotNone(match, "CurUninstallStepChanged is missing")
        self.text = text
        self.step = match.group(0)

    def at(self, needle):
        self.assertIn(needle, self.step, needle)
        return self.step.index(needle)

    def test_it_runs_once_before_any_file_is_removed(self):
        self.assertIn("CurUninstallStep <> usUninstall", self.step)
        self.assertNotIn("usPostUninstall", self.step)

    def test_the_steps_come_in_this_order(self):
        order = [self.at("'uninstall'"), self.at("EndRunningPrograms(True)"), self.at("'stop'"), self.at("EndRunningPrograms(False)"),
                 self.at("RemoveStartAtSignIn"), self.at("RemoveAppFromPath")]
        self.assertEqual(order, sorted(order))

    def test_the_path_is_changed_only_when_it_changes_and_as_expandable_text(self):
        self.assertIn("RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path'", self.text)
        self.assertNotIn("RegWriteStringValue(HKCU, EnvironmentKey", self.text)
        self.assertGreaterEqual(self.text.count("if Updated <> Old then") + self.text.count("if Updated = Old then"), 2)
        self.assertIn("ChangesEnvironment=yes", self.text)
        self.assertNotIn("{olddata}", self.text, "the old append-only Path entry is gone")

    def test_the_start_at_sign_in_name_and_key_are_the_ones_one_mk_install_writes(self):
        install = (REPO / "crates/ctl/src/install.rs").read_text(encoding="utf-8")
        run_key = re.search(r'const RUN_KEY: &str = r"HKCU\\([^"]+)"', install).group(1)
        self.assertIn(f"RunKey = '{run_key}'", self.text)
        self.assertIn('"/v", "one-mk"', install)
        self.assertIn("RegDeleteValue(HKCU, RunKey, 'one-mk')", self.text)

    def test_it_does_not_end_programs_by_name_alone(self):
        self.assertNotIn("taskkill", self.text.lower())
        self.assertIn("ExecutablePath.StartsWith(", self.text)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0], *sys.argv[1:]], verbosity=1)
