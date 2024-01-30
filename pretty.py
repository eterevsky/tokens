import json
import sys

def serialize(data, maxlen=80):
    try_short = json.dumps(data, indent=None, ensure_ascii=False)
    if len(try_short) <= maxlen or type(data) not in (list, dict):
        yield try_short
        return
    if type(data) is list:
        yield '['
        last_line = None
        for item in data:
            if last_line is not None:
                yield last_line + ','
                last_line = None
            for subline in serialize(item, maxlen - 2):
                if last_line is not None:
                    yield last_line
                last_line = '  ' + subline
        if last_line is not None:
            yield last_line
        yield ']'
    elif type(data) is dict:
        yield '{'
        last_line = None
        for key, value in data.items():
            if last_line is not None:
                yield last_line + ','
                last_line = None

            key_repr = json.dumps(key, indent=None, ensure_ascii=False)
            short_value = json.dumps(value, indent=None, ensure_ascii=False)

            short_line = '  ' + key_repr + ': ' + short_value
            if len(short_line) <= maxlen + 1:
                last_line = short_line
            else:
                for subline in serialize(value, maxlen - 2):
                    if last_line is None:
                        last_line = '  ' + key_repr + ': ' + subline
                    else:
                        yield last_line
                        last_line = '  ' + subline
        if last_line is not None:
            yield last_line
        yield '}'


def save_json(data, wfile):
    for line in serialize(data):
        wfile.write(line + '\n')


with open(sys.argv[1], "r", encoding="utf-8") as f: 
    d = json.load(f)

with open(sys.argv[1], "w", encoding="utf-8") as f:
    save_json(d, f)
