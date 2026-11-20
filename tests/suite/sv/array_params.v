// expected
// Unpacked array parameters (IEEE 1800-2017 6.20.2): patterns (nested, keys, default), struct
// elements, multi-dimensional, in constant expressions and generate conditions, as an
// override from a package parameter, read with variable indexes at run time.
package pk;
  typedef struct packed { logic [3:0] op; logic [7:0] mask; } ent_t;
  parameter ent_t TABLE [3] = '{'{op: 1, mask: 8'h0f}, '{op: 2, mask: 8'hf0}, '{op: 3, mask: 8'hff}};
endpackage
module sub #(parameter int N = 2, parameter pk::ent_t T [N] = '{default: '0}) (input [3:0] sel, output logic [7:0] m);
  always_comb begin
    m = 0;
    for (int i = 0; i < N; i++) if (T[i].op == sel) m = T[i].mask;
  end
  for (genvar g = 0; g < N; g++) begin : gen
    if (T[g].op == 2) initial $display("entry %0d has op 2", g);
  end
endmodule
module t;
  localparam int A [4] = '{10, 20, 30, 40};
  localparam logic [7:0] B [2][2] = '{'{1, 2}, '{3, 4}};
  localparam int C [0:3] = '{1: 7, default: 5};
  localparam int S = A[1] + A[3];
  logic [3:0] sel;
  logic [7:0] m;
  sub #(.N(3), .T(pk::TABLE)) u(.sel(sel), .m(m));
  integer i;
  initial begin
    $display("%0d %0d %0d %0d S=%0d", A[0], A[1], A[2], A[3], S);
    $display("%0d %0d %0d %0d", B[0][0], B[0][1], B[1][0], B[1][1]);
    $display("%0d %0d %0d %0d $size=%0d", C[0], C[1], C[2], C[3], $size(A));
    for (i = 0; i < 4; i++) begin sel = i; #1 $display("sel=%0d m=%h", sel, m); end
  end
endmodule
