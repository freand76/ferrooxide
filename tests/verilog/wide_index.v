// An index wider than 32 bits counts by its low 32 bits (as the reference, ICARUS_NOTES 13);
// it once overflowed the range check of a part select and crashed.
module top;
  reg [3:0] array [1:8];
  reg [127:0] b;
  reg [63:0] d;
  reg signed [127:0] sb;
  reg [15:0] v;
  reg [3:0] w;
  initial begin
    array[7] = 4'b1001;
    v = 16'h1234;
    b = 7; b[100] = 1; $display("%b", array[b]);
    b = 7; b[31] = 1; $display("%b", array[b]);
    b = 7; b[100] = 1'bx; $display("%b", array[b]);
    d = 7; d[40] = 1; $display("%b", array[d]);
    sb = 7; sb[127] = 1; $display("%b", array[sb]);
    b = 4; b[70] = 1; $display("%b %b", v[b], v[b +: 4]);
    w = v[b +: 4];
    $display("%b", w);
    b = 7; b[64] = 1; array[b] = 4'b0110; $display("%b", array[7]);
    b[63] = 1; w = v[b +: 4]; $display("%b", w);
  end
endmodule
