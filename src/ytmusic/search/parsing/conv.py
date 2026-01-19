# convert consts from https://github.com/sigma67/ytmusicapi/blob/main/ytmusicapi/navigation.py to rust format.
# flattens the constants, in that lists that extended other lists contains the other list by default.
#
# to use:
# 1. download the above file and rename to "consts.py".
# 2. move it to the same dir as this script.
# 3. run this script.

import consts

consts = [(x, y) for (x, y) in vars(consts).items() if not x.startswith("_")]

output = ""

for (name, value) in consts:
    # if it's not a string, we join it by `/`s
    if type(value) is not str:
        value = [str(x) for x in value]
        value = "/".join(value)
    output += f"pub const {name}: &str = \"/{value}\";\n"

with open("consts.rs", "w") as f:
    f.write(output)