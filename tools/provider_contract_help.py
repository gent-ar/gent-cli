from __future__ import annotations

import re


SECTION = re.compile(r"^(Commands|Options|Arguments):\s*$")
OPTION = re.compile(r"^ {2,8}(?:(-[A-Za-z0-9]), )?(--[A-Za-z0-9][A-Za-z0-9-]*)(?: ([<\[][^>\]]*[>\]]))?(?=\s|$)")
SHORT_ONLY = re.compile(r"^ {2,8}(-[A-Za-z0-9])(?: ([<\[][^>\]]*[>\]]))?(?=\s|$)")
COMMAND = re.compile(r"^ {2}([a-z][a-z0-9-]*(?:\|[a-z][a-z0-9-]*)*)(?: \[options\])?(?:\s|$)")
COMMANDER_CHOICES = re.compile(r"\(choices: ((?:\"[^\"]*\"(?:, )?)+)")
CLAP_CHOICES = re.compile(r"\[possible values: ([^\]]+)\]")
CLAP_ALIASES = re.compile(r"\[aliases: ([^\]]+)\]")


def argument_shape(token: str | None) -> str:
    if token is None:
        return "none"
    variadic = token.endswith("...>") or token.endswith("...]")
    kind = "required" if token.startswith("<") else "optional"
    return f"{kind}-variadic" if variadic else kind


def choices(text: str) -> list[str]:
    commander = COMMANDER_CHOICES.search(text)
    if commander:
        return sorted(re.findall(r"\"([^\"]*)\"", commander.group(1)))
    clap = CLAP_CHOICES.search(text)
    if clap:
        return sorted(value.strip() for value in clap.group(1).split(","))
    return []


def usage(text: str) -> str:
    for line in text.splitlines():
        if line.startswith("Usage:"):
            return " ".join(line.split()[1:])
    return ""


def finish(options: dict[str, dict[str, object]], current: tuple[str, list[str]] | None) -> None:
    if current is None:
        return
    flag, description = current
    values = choices(" ".join(" ".join(description).split()))
    if values:
        options[flag]["choices"] = values


def parse_help(text: str) -> dict[str, object]:
    section = None
    options: dict[str, dict[str, object]] = {}
    subcommands: dict[str, list[str]] = {}
    current: tuple[str, list[str]] | None = None
    for line in text.splitlines():
        header = SECTION.match(line)
        if header:
            finish(options, current)
            current = None
            section = header.group(1)
            continue
        if section == "Options":
            option = OPTION.match(line) or SHORT_ONLY.match(line)
            if option:
                finish(options, current)
                if option.re is OPTION:
                    short, flag, token = option.groups()
                else:
                    short, flag, token = None, option.group(1), option.group(2)
                options[flag] = {"arg": argument_shape(token), "short": short}
                current = (flag, [line[option.end():]])
            elif current is not None:
                current[1].append(line)
        elif section == "Commands":
            command = COMMAND.match(line)
            if command:
                names = command.group(1).split("|")
                aliases = CLAP_ALIASES.search(line)
                extra = [value.strip() for value in aliases.group(1).split(",")] if aliases else []
                subcommands[names[0]] = sorted(names[1:] + extra)
    finish(options, current)
    return {"usage": usage(text), "options": dict(sorted(options.items())), "subcommands": dict(sorted(subcommands.items()))}
