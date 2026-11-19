// Assignment patterns for unpacked arrays (IEEE 1800-2017 10.9.1): positional items from
// the left, index keys (also a parameter), `default:` (to every element below it, or an
// array value for a subarray), nested patterns for further dimensions, unpacked structs as
// elements; and slices of unpacked arrays as array values (7.4.6).
module t;
  integer errors = 0;
  localparam K = 2;
  typedef struct { int a; int b; } ms_t;
  int a[4] = '{1, 2, 3, 4};
  int b[3:0];
  int m[2][3];
  ms_t ms[1:0] = '{'{0, 5}, '{1, 1}};
  logic [7:0] w[0:3];
  bit c[7:0], d[7:0];
  real r[1:0][0:1], r2[1:0][0:1];
  string s[2];
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, {a[0], a[3]}, {32'd1, 32'd4});
    b = '{5, 6, 7, 8};                       // b[3] is the leftmost element
    check(`__LINE__, {b[3], b[0]}, {32'd5, 32'd8});
    m = '{'{1, 2, 3}, '{4, 5, 6}};
    check(`__LINE__, {m[0][2], m[1][0]}, {32'd3, 32'd4});
    m = '{default: 9};
    check(`__LINE__, {m[0][0], m[1][2]}, {32'd9, 32'd9});
    m[1] = '{0: 7, K: 8, default: 1};
    check(`__LINE__, {m[1][0], m[1][1], m[1][2]}, {32'd7, 32'd1, 32'd8});
    m = '{1: '{default: 3}, default: a[1:3]};
    check(`__LINE__, {m[0][0], m[0][2], m[1][1]}, {32'd2, 32'd4, 32'd3});
    check(`__LINE__, {ms[1].b, ms[0].a}, {32'd5, 32'd1});
    w = '{default: 8'hx};
    check(`__LINE__, w[2], 8'hxx);
    c = '{1, 1, 1, 1, 0, 0, 0, 0};
    d = '{default: 0};
    d[5:3] = c[7:5];                         // a slice from a slice
    check(`__LINE__, {d[6], d[5], d[3], d[2]}, 4'b0110);
    d[1+:2] = c[7-:2];
    check(`__LINE__, {d[2], d[1], d[0]}, 3'b110);
    r = '{'{1.5, 2}, '{3.25, 4}};           // reals: r[1][0] is the first
    r2 = r;
    if (r2[1][0] != 1.5 || r2[0][1] != 4.0) begin $display("FAILED line %0d", `__LINE__); errors++; end
    s = '{"ab", "cd"};
    if (s[1] != "cd") begin $display("FAILED line %0d", `__LINE__); errors++; end
    if (errors == 0) $display("PASSED");
  end
endmodule
