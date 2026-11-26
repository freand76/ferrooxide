// compound assignments and increments of a target whose indexes have side effects
// (IEEE 1800-2017 11.4.1): the indexes are evaluated once
module top;
  logic [3:0] x [0:2];
  int m [3][3];
  logic [15:0] v;
  logic [1:0] i;
  int j, k;
  int fails = 0;
  initial begin
    x = '{1, 1, 1};
    i = 0;
    x[i++] += 2;
    if (x[0] !== 3 || x[1] !== 1 || i !== 1) fails++;
    x[i++]++;
    if (x[1] !== 2 || i !== 2) fails++;
    x[--i] <<= 1;
    if (x[1] !== 4 || i !== 1) fails++;
    m = '{default: 10};
    j = 0;
    k = 2;
    m[j++][k--] -= 3;
    if (m[0][2] != 7 || m[1][1] != 10 || j != 1 || k != 1) fails++;
    v = 16'h00f0;
    k = 4;
    v[k++ +: 4] |= 4'h3;
    if (v !== 16'h00f0 || k != 5) fails++;
    v[k++ +: 4] ^= 4'hf;
    if (v !== 16'h0110 || k != 6) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule
