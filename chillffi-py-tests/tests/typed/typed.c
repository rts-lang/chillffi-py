#include <stdint.h>
#include <stddef.h>

#ifdef _WIN32
  #define ChillffiExport __declspec(dllexport)
#else
  #define ChillffiExport
#endif

// Signed width: returns an `int32`; read as an `i64` (`inferred`) the result of a
// negative value carries garbage in the high bits — `Typed(I32)` is required.
ChillffiExport int32_t neg32(int32_t x)
{
  return -x;
}

// Unsigned width: `255 + 1 = 0` in a `uint8` — proves the argument went out as a `u8`.
ChillffiExport uint8_t roll8(uint8_t x)
{
  return (uint8_t)(x + 1);
}

// `float` parameter: `float` vs `double` — different argument widths (xmm registers).
ChillffiExport float halff(float x)
{
  return x / 2.0f;
}

// `double` parameter: the same width story, 64-bit this time.
ChillffiExport double twiced(double x)
{
  return 2.0 * x;
}

// `_Bool` is a separate ABI type.
ChillffiExport int truthy(_Bool b)
{
  return b ? 1 : 0;
}

// `const char*, size_t` — a (`ptr`, `len`) pair: chillffi.String passes one python
// argument, `chillffi` unpacks it into TWO C arguments (`Value::String`).
ChillffiExport size_t countBytes(const char *s, size_t len)
{
  (void)s;
  return len;
}

// The first byte of the same pair, or 0 for an empty string.
ChillffiExport int8_t firstByte(const char *s, size_t len)
{
  return len > 0 ? (int8_t)s[0] : 0;
}

// Pointers: write into the clone's memory by address;
// NULL is distinguished honestly.
ChillffiExport void fill(char *p, size_t n, char c)
{
  for (size_t i = 0; i < n; i++) {
    p[i] = c;
  }
}

// Reports whether the pointer is NULL.
ChillffiExport int isNull(const void *p)
{
  return p == NULL;
}
