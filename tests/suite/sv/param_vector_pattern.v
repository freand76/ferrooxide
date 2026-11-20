// Assignment patterns as values of vector parameters: one item per bit, each converted to
// the bit type (IEEE 1800-2017 10.9.1); `default:` and index keys.
module t;
  localparam bit [2:0] A = '{1'b1, 2.0, 2 + 1};
  localparam logic signed [3:0] B = '{1, 0, 1, 1};
  localparam bit [7:0] C = '{default: 1};
  localparam bit [3:0] D = '{0: 1, 3: 1, default: 0};
  localparam bit [0:3] E = '{1, 0, 0, 0};
  localparam bit [1:0][3:0] F = '{4'h1, 4'h2};
  parameter logic [3:0] G = '{1'bx, 1'bz, 0, 1};
  localparam int K = 2;
  bit [3:0] v = '{K: 1, default: 0};
  logic [3:1] w;
  integer fails = 0;
  task check(input [63:0] got, input [63:0] want, input [8*8-1:0] what);
    if (got !== want) begin
      $display("FAILED: %0s = %b, want %b", what, got, want);
      fails = fails + 1;
    end
  endtask
  initial begin
    check(A, 3'b101, "A");
    check(B, 64'hffff_ffff_ffff_fffb, "B");
    check(C, 8'hff, "C");
    check(D, 4'b1001, "D");
    check(E, 4'b1000, "E");
    check(F, 8'h12, "F");
    check(v, 4'b0100, "v");
    w = '{1: 1'b1, 3: 1'b0, 2: 1'b1};
    check(w, 3'b011, "w");
    if (G !== 4'bxz01) begin
      $display("FAILED: G = %b", G);
      fails = fails + 1;
    end
    if (fails == 0) $display("PASSED");
  end
endmodule
