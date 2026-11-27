// A class's parameter in a packed range, and one of its types as an associative array's
// index type (IEEE 1800-2017 8.23, 7.8.4).
class SFR;
  localparam int w = 8;
  typedef logic [3:0] nib_t;
endclass
module top;
  wire [SFR::w-1:0] a;
  int m [SFR::nib_t];
  logic [SFR::w:1] b;
  initial begin
    m[4'h3] = 5;
    if ($bits(a) == 8 && $bits(b) == 8 && m.num() == 1) $display("PASSED"); else $display("FAILED");
  end
endmodule
