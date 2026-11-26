// an element of a packed array of an enum is a value of the enum (IEEE 1800-2017 6.19,
// 7.4): it may be assigned to an enum variable without a cast
module top;
  typedef enum logic [1:0] { e_I, e_S, e_M } st_e;
  st_e [3:0] arr;
  st_e s, t;
  logic [1:0] k;
  initial begin
    arr = '{e_M, e_S, e_I, e_S};
    k = 1;
    s = arr[k];
    t = k[0] ? arr[3] : e_I;
    if (s == e_I && t == e_M && s.name() == "e_I") $display("PASSED");
    else $display("FAILED");
  end
endmodule
