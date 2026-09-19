# Minimised from aiobotocore and celery, where all of these were reported as
# quantities that are not numbers.
#
# Two things are going on. Some are questions - a name that asks one is a
# question whatever it ends with. The rest are verb phrases: prepend_size and
# populate_content_length describe an action, and the boolean is its outcome.
# A boolean is never a broken promise about a number.
should_remove_content_length = True
is_exceeds_max_size = False
has_total = True
prepend_size = True
automatically_set_content_length = False


def populate_content_length() -> bool:
    return True
