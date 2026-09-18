# Exercises the comment syntax. No checker exists yet to satisfy these, so they
# are parsed rather than run - see tests/fixture_syntax.rs.
x = f()          # expect: C1
y = g()          # expect: C1, C3b
z = h()
