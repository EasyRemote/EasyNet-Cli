#!/usr/bin/env python3
import os
import runpy
import sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
target = os.path.join(root, "engineering", "scripts", "rewrite-rfc-messages-round2.py")
sys.argv[0] = target
runpy.run_path(target, run_name="__main__")
