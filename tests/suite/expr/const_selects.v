// Part-select bounds and the selects in continuous assignment targets are constant
// expressions (IEEE 1800-2017 11.5.1, 10.3.1): parameters, genvars, constant function calls
// and queries like $bits are; a run-time index uses an indexed part-select.
module t;
  localparam W = 8;
  function integer half(input integer n);
    half = n / 2;
  endfunction
  reg [15:0] v = 16'hbeef;
  wire [7:0] lo, hi;
  wire [3:0] nib [0:3];
  assign lo = v[W-1:0];
  assign hi = v[$bits(v)-1:half($bits(v))];
  for (genvar i = 0; i < 4; i++) begin : g
    assign nib[i] = v[4*i+3:4*i];
  end
  integer k;
  reg [3:0] r;
  initial begin
    #1;
    k = 2;
    r = v[k*4 +: 4];
    if (lo === 8'hef && hi === 8'hbe && nib[3] === 4'hb && r === 4'he) $display("PASSED");
    else $display("FAILED: %h %h %h %h", lo, hi, nib[3], r);
  end
endmodule
