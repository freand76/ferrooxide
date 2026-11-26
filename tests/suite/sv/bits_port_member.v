// $bits of a member of a struct port in a parameter value: a constant (IEEE 1800-2017
// 20.6.2)
typedef struct packed { logic [5:0] stride; logic [3:0] nblocks; } p_t;
module m (input p_t csr_param_i, output logic [7:0] o);
  localparam int SW = $bits(csr_param_i.stride);
  localparam int NW = $bits(csr_param_i.nblocks);
  logic [SW-1:0] s;
  initial #1 if (SW == 6 && NW == 4 && $bits(s) == 6 && $bits(csr_param_i) == 10) $display("PASSED");
  else $display("FAILED");
endmodule
module top;
  p_t p;
  logic [7:0] o;
  m u(p, o);
endmodule
