//go:build !unix

package instance

// alive cannot ask about a pid here; the address check in Running decides.
func alive(int) bool { return true }
