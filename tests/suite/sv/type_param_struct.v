// Members and assignment patterns of variables whose type is a type parameter (IEEE 1800
// 6.20.3, 7.2, 10.9.2): the members come from the type the instance is given.
typedef struct packed {
  logic [3:0] tag;
  logic [7:0] data;
} pkt_t;

typedef struct packed {
  logic signed [5:0] v;
  pkt_t p;
} rec_t;

module holder #(parameter type T = pkt_t) (output logic [$bits(T)-1:0] flat);
  T x;
  T arr [0:1];
  initial begin
    x = '{default: '0};
    x.tag = 4'hA;
    x.data = 8'h5C;
    arr[0] = '{tag: 4'h1, data: 8'h23};
    arr[1] = x;
    arr[1].data[3:0] = 4'hF;
    flat = x;
  end
endmodule

module rec_holder #(parameter type T = rec_t) (output logic [$bits(T)-1:0] flat, output logic neg);
  T r;
  initial begin
    r = '{v: -3, p: '{tag: 4'h2, data: 8'h81}};
    r.p.tag = r.p.tag + 1;
    neg = r.v < 0;
    flat = r;
  end
endmodule

module t;
  integer errors = 0;
  wire [11:0] f1;
  wire [17:0] f2;
  wire n;
  localparam pkt_t P = '{tag: 4'h7, data: 8'h99};
  holder #(.T(pkt_t)) h (.flat(f1));
  rec_holder #(.T(rec_t)) r (.flat(f2), .neg(n));
  function automatic pkt_t swap(pkt_t a);
    swap = '{tag: a.data[3:0], data: {4'h0, a.tag}};
  endfunction
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, f1, 12'hA5C);
    check(`__LINE__, h.arr[0], 12'h123);
    check(`__LINE__, h.arr[1], 12'hA5F);
    check(`__LINE__, f2, {6'b111101, 4'h3, 8'h81});
    check(`__LINE__, n, 1);
    check(`__LINE__, P.data, 8'h99);
    check(`__LINE__, swap(P), 12'h907);
    check(`__LINE__, swap('{tag: 4'h1, data: 8'h02}), 12'h201);
    if (errors == 0) $display("PASSED");
    $finish;
  end
endmodule
