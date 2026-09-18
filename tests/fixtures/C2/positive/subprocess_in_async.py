import subprocess


async def build():
    subprocess.run(["make"])  # expect: C2
