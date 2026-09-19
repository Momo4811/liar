class Client:
    def fetch(self, url):
        """Fetch a thing.

        Args:
            url: where to fetch from
        """
        return get(url)
